//! ChatService — отправка, история, удаление чатов.
//!
//! Центральный сервис для работы с сообщениями.
//! Знает о шифровании, очереди pending и удалении с IMAP.

use chrono::Utc;
use uuid::Uuid;

use crate::{
    Error, Result,
    events::{ChatEvent, EventBus},
    models::{
        conversation::Conversation,
        message::{Message, MessageKind, MessageStatus},
    },
    ports::{
        email::{DynEmailTransport, OutgoingEmail},
        storage::{CreateMessage, DynStorage},
    },
    services::account::AccountService,
};

pub struct ChatService {
    storage: DynStorage,
    email: DynEmailTransport,
    account_svc: AccountService,
    events: EventBus,
    /// URL для скачивания приложения — вставляется в invite-письма
    app_download_url: String,
}

impl ChatService {
    pub fn new(
        storage: DynStorage,
        email: DynEmailTransport,
        account_svc: AccountService,
        events: EventBus,
        app_download_url: String,
    ) -> Self {
        Self {
            storage,
            email,
            account_svc,
            events,
            app_download_url,
        }
    }

    // ── Отправка ─────────────────────────────────────────────────────────────

    /// Отправляет текстовое сообщение.
    ///
    /// Автоматически:
    /// - Создаёт беседу если её нет
    /// - Шифрует (или ставит в очередь если контакт без приложения)
    /// - Отправляет через SMTP
    /// - Сохраняет в БД
    pub async fn send_message(
        &self,
        account_id: Uuid,
        contact_id: Uuid,
        body: String,
        reply_to: Option<Uuid>,
    ) -> Result<Message> {
        let account = self.storage.get_account(account_id).await?;
        let contact = self.storage.get_contact(contact_id).await?;

        // Находим или создаём беседу
        let conv = self
            .get_or_create_direct_conversation(account_id, contact_id)
            .await?;

        let msg_id = Uuid::new_v4();
        let sent_at = Utc::now();

        // Определяем статус и пробуем отправить
        let (status, imap_uid, imap_folder) = match &contact.public_keys {
            None => {
                // Контакт без приложения — ставим в очередь и отправляем invite
                self.send_invite(&account.email, &contact.email, &body)
                    .await?;
                (MessageStatus::Queued, None, None)
            }
            Some(their_keys) => {
                // Контакт активен — шифруем и отправляем
                let ciphertext = self
                    .encrypt_direct(
                        account_id, their_keys, conv.id, msg_id, &body, reply_to, &sent_at,
                    )
                    .await?;

                self.email
                    .send(OutgoingEmail {
                        from: account.email.clone(),
                        to: vec![contact.email.clone()],
                        subject: encryption::disguise::random_subject(),
                        body: ciphertext,
                        extra_headers: vec![("X-EChat".into(), "1".into())],
                    })
                    .await?;

                (MessageStatus::Sent, None, None)
            }
        };

        // Сохраняем в БД
        self.storage
            .create_message(CreateMessage {
                id: msg_id,
                conversation_id: conv.id,
                account_id,
                from_email: account.email.clone(),
                body: Some(body.clone()),
                kind: MessageKind::Text,
                status: status.clone(),
                reply_to,
                imap_uid: imap_uid.clone(),
                imap_folder: imap_folder.clone(),
                sent_at,
            })
            .await?;

        // Обновляем превью беседы
        self.storage
            .update_conversation_last_message(conv.id, body.clone(), sent_at, false)
            .await?;

        let message = Message {
            id: msg_id,
            conversation_id: conv.id,
            account_id,
            from_email: account.email,
            body: Some(body),
            kind: MessageKind::Text,
            status,
            reply_to,
            imap_uid,
            imap_folder,
            sent_at,
            received_at: None,
        };

        Ok(message)
    }

    // ── История ──────────────────────────────────────────────────────────────

    /// Возвращает историю беседы с пагинацией.
    pub async fn get_history(
        &self,
        conv_id: Uuid,
        before: Option<chrono::DateTime<Utc>>,
        limit: usize,
    ) -> Result<Vec<Message>> {
        self.storage
            .get_message_history(conv_id, before, limit)
            .await
    }

    /// Список всех бесед аккаунта.
    pub async fn list_conversations(&self, account_id: Uuid) -> Result<Vec<Conversation>> {
        self.storage.list_conversations(account_id).await
    }

    /// Помечает беседу как прочитанную.
    pub async fn mark_read(&self, conv_id: Uuid) -> Result<()> {
        self.storage.mark_conversation_read(conv_id).await
    }

    // ── Удаление ─────────────────────────────────────────────────────────────

    /// Удаляет беседу: сначала письма с IMAP сервера, затем из БД.
    ///
    /// `delete_from_server` — если true, удаляем письма с почтового сервера.
    /// Рекомендуется всегда true для приватности.
    pub async fn delete_conversation(&self, conv_id: Uuid, delete_from_server: bool) -> Result<()> {
        if delete_from_server {
            // Получаем все IMAP UID
            let uid_entries = self.storage.get_imap_uids_for_deletion(conv_id).await?;

            // Группируем по папкам
            let mut by_folder: std::collections::HashMap<String, Vec<u32>> =
                std::collections::HashMap::new();
            for entry in uid_entries {
                by_folder.entry(entry.folder).or_default().push(entry.uid);
            }

            // Удаляем с IMAP пачками по папкам
            for (folder, uids) in by_folder {
                if let Err(e) = self.email.delete_messages(&folder, &uids).await {
                    // Логируем но не прерываем — продолжаем удалять из БД
                    tracing::warn!("Не удалось удалить письма с IMAP ({}): {}", folder, e);
                }
            }
        }

        // Удаляем из БД (CASCADE удалит сообщения)
        self.storage.delete_conversation(conv_id).await?;
        tracing::info!("Беседа {} удалена", conv_id);
        Ok(())
    }

    // ── Входящие сообщения ────────────────────────────────────────────────────

    /// Обрабатывает входящее расшифрованное сообщение.
    ///
    /// Вызывается из `sync::processor` после успешной расшифровки.
    pub async fn handle_incoming(
        &self,
        account_id: Uuid,
        from_email: String,
        _conv_id: Uuid,
        msg_id: Uuid,
        body: String,
        sent_at: chrono::DateTime<Utc>,
        imap_uid: u32,
        imap_folder: String,
    ) -> Result<()> {
        // Дедупликация
        if self.storage.message_exists(msg_id).await? {
            tracing::debug!("Дубликат сообщения {}, пропускаем", msg_id);
            return Ok(());
        }

        // Находим контакт по email
        let contact = self
            .storage
            .get_contact_by_email(account_id, &from_email)
            .await?;

        // Находим беседу
        let conv = match self
            .storage
            .find_direct_conversation(account_id, contact.id)
            .await?
        {
            Some(c) => c,
            None => {
                // Создаём беседу автоматически
                self.get_or_create_direct_conversation(account_id, contact.id)
                    .await?
            }
        };

        let now = Utc::now();

        self.storage
            .create_message(CreateMessage {
                id: msg_id,
                conversation_id: conv.id,
                account_id,
                from_email: from_email.clone(),
                body: Some(body.clone()),
                kind: MessageKind::Text,
                status: MessageStatus::Delivered,
                reply_to: None,
                imap_uid: Some(imap_uid),
                imap_folder: Some(imap_folder),
                sent_at,
            })
            .await?;

        self.storage
            .update_conversation_last_message(conv.id, body.clone(), sent_at, true)
            .await?;

        let message = Message {
            id: msg_id,
            conversation_id: conv.id,
            account_id,
            from_email,
            body: Some(body),
            kind: MessageKind::Text,
            status: MessageStatus::Delivered,
            reply_to: None,
            imap_uid: Some(imap_uid),
            imap_folder: None,
            sent_at,
            received_at: Some(now),
        };

        self.events.emit(ChatEvent::NewMessage {
            conversation_id: conv.id,
            message,
        });

        Ok(())
    }

    // ── Внутренние методы ─────────────────────────────────────────────────────

    async fn get_or_create_direct_conversation(
        &self,
        account_id: Uuid,
        contact_id: Uuid,
    ) -> Result<Conversation> {
        if let Some(conv) = self
            .storage
            .find_direct_conversation(account_id, contact_id)
            .await?
        {
            return Ok(conv);
        }

        let conv_id = Uuid::new_v4();
        self.storage
            .create_direct_conversation(conv_id, account_id, contact_id)
            .await?;
        self.storage.get_conversation(conv_id).await
    }

    /// Шифрует сообщение для direct-чата.
    async fn encrypt_direct(
        &self,
        account_id: Uuid,
        their_keys: &crate::models::contact::ContactPublicKeys,
        conv_id: Uuid,
        msg_id: Uuid,
        body: &str,
        reply_to: Option<Uuid>,
        sent_at: &chrono::DateTime<Utc>,
    ) -> Result<String> {
        let keypair = self.account_svc.load_or_create_keypair(account_id).await?;

        let their_x25519 = their_keys
            .x25519_bytes()
            .ok_or_else(|| Error::Encryption("Некорректный X25519 ключ".into()))?;

        let shared_secret = encryption::session::derive_from_bytes(
            keypair.secret_key(),
            &their_x25519,
            "direct-chat",
        )
        .map_err(|e| Error::Encryption(e.to_string()))?;

        // Формируем ChatEnvelope (содержимое до шифрования)
        let envelope = serde_json::json!({
            "msg_id": msg_id,
            "conv_id": conv_id,
            "kind": "text",
            "sent_at": sent_at.to_rfc3339(),
            "body": body,
            "reply_to": reply_to,
            "protocol_version": 1,
        });
        let envelope_bytes =
            serde_json::to_vec(&envelope).map_err(|e| Error::Internal(e.to_string()))?;

        let payload = encryption::cipher::encrypt(&envelope_bytes, &shared_secret)
            .map_err(|e| Error::Encryption(e.to_string()))?;

        Ok(payload.to_base64())
    }

    /// Отправляет invite-письмо (контакт без приложения).
    async fn send_invite(&self, from: &str, to: &str, _body: &str) -> Result<()> {
        // Создаём placeholder payload — расшифровать невозможно без ключа
        // После handshake queued сообщения будут отправлены повторно
        let invite_body = format!(
            "Привет!\n\nЯ использую защищённый мессенджер EChat.\n\
            Установи приложение: {}\n\n\
            После установки ты увидишь моё сообщение.\n\n---\n\
            [зашифрованное сообщение]",
            self.app_download_url
        );

        self.email
            .send(OutgoingEmail {
                from: from.to_string(),
                to: vec![to.to_string()],
                subject: "Привет! Я пишу через EChat".to_string(),
                body: invite_body,
                extra_headers: vec![("X-EChat".into(), "1".into())],
            })
            .await
    }
}
