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
        storage::{ConversationKeys, CreateMessage, DynStorage},
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

        // Логирование для отладки
        tracing::info!(
            "send_message: contact={}, status={:?}, has_keys={}",
            contact.email,
            contact.status,
            contact.public_keys.is_some()
        );
        if let Some(ref keys) = contact.public_keys {
            tracing::info!(
                "  x25519: {:x?}..., ed25519: {:x?}...",
                &keys.x25519[..4],
                &keys.ed25519[..4]
            );
        }

        // Находим или создаём беседу
        let conv = self
            .get_or_create_direct_conversation(account_id, contact_id)
            .await?;

        let msg_id = Uuid::new_v4();
        let sent_at = Utc::now();

        // Получаем ключи диалога И контакта (fallback)
        let conv_keys = self.storage.get_conversation_keys(conv.id).await.ok();
        let contact = self.storage.get_contact(contact_id).await?;

        // Определяем статус и пробуем отправить
        let (status, imap_uid, imap_folder, error_message) = if let Some(keys) = &conv_keys {
            // Есть ключи диалога
            match (&keys.my_keypair_json, &keys.their_public_key_json) {
                (Some(_), None) => {
                    // Ключи диалога созданы, но публичный ключ собеседника ещё не установлен
                    // Ставим в очередь и отправляем invite
                    tracing::warn!("their_public_key_json = None, отправляем invite");
                    self.send_invite(&account.email, &contact.email, &body)
                        .await?;
                    (MessageStatus::Queued, None, None, None)
                }
                (Some(my_keys_json), Some(their_key_json)) => {
                    // Ключи диалога активны — шифруем и отправляем
                    tracing::info!("Ключи диалога активны, шифруем сообщение...");
                    tracing::debug!("my_keypair_json (первые 50): {:?}", my_keys_json.chars().take(50).collect::<String>());
                    tracing::debug!("their_key_json (первые 50): {:?}", their_key_json.chars().take(50).collect::<String>());

                    // their_key_json хранится как JSON PublicKeys (сериализованный)
                    let their_keys: encryption::keypair::PublicKeys =
                        serde_json::from_str(their_key_json)
                            .map_err(|e| {
                                tracing::error!("Ошибка парсинга their_key_json={:?}: {}", their_key_json, e);
                                Error::Internal(format!("Ошибка парсинга ключей: {}", e))
                            })?;

                    let ciphertext = self
                        .encrypt_direct_with_keys(
                            my_keys_json,
                            &their_keys,
                            conv.id,
                            msg_id,
                            &body,
                            reply_to,
                            &sent_at,
                            account_id,
                        )
                        .await?;

                    tracing::info!("Сообщение зашифровано, размер: {} байт", ciphertext.len());

                    // Убеждаемся что папка EChat существует
                    tracing::info!("Проверяем/создаём папку EChat на сервере...");
                    match self.email.ensure_echat_folder().await {
                        Ok(()) => tracing::info!("Папка EChat готова"),
                        Err(e) => tracing::warn!("Не удалось создать папку EChat: {}", e),
                    }

                    tracing::info!("Отправляем письмо через SMTP: {} → {}", account.email, contact.email);

                    // Отправляем через SMTP
                    match self
                        .email
                        .send(OutgoingEmail {
                            from: account.email.clone(),
                            to: vec![contact.email.clone()],
                            subject: encryption::disguise::random_subject(),
                            body: ciphertext,
                            extra_headers: vec![("X-EChat".into(), "1".into())],
                        })
                        .await
                    {
                        Ok(()) => {
                            tracing::info!("Письмо успешно отправлено через SMTP");
                            (MessageStatus::Sent, None, None, None)
                        }
                        Err(e) => {
                            tracing::error!("Ошибка SMTP: {}", e);
                            (MessageStatus::Failed, None, None, Some(e.to_string()))
                        }
                    }
                }
                _ => {
                    // Ключи диалога не полные — пробуем fallback на ключи контакта
                    tracing::warn!("Ключи диалога не полные, используем ключи контакта");
                    self.send_with_contact_keys(
                        account_id,
                        &account,
                        &contact,
                        conv.id,
                        msg_id,
                        body.clone(),
                        reply_to,
                        sent_at,
                    )
                    .await?
                }
            }
        } else {
            // Ключей диалога нет — используем старый механизм с ключами контакта
            self.send_with_contact_keys(
                account_id,
                &account,
                &contact,
                conv.id,
                msg_id,
                body.clone(),
                reply_to,
                sent_at,
            )
            .await?
        };

        // Сохраняем в БД (всегда, даже если ошибка отправки)
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
                error_message: error_message.clone(),
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

    /// Отправляет сообщение используя ключи контакта (fallback механизм).
    async fn send_with_contact_keys(
        &self,
        account_id: Uuid,
        account: &crate::models::account::Account,
        contact: &crate::models::contact::Contact,
        conv_id: Uuid,
        msg_id: Uuid,
        body: String,
        reply_to: Option<Uuid>,
        sent_at: chrono::DateTime<Utc>,
    ) -> Result<(MessageStatus, Option<u32>, Option<String>, Option<String>)> {
        // Проверяем есть ли у контакта публичные ключи
        match &contact.public_keys {
            None => {
                // Контакт без приложения — отправляем invite
                tracing::warn!("contact.public_keys = None, отправляем invite");
                self.send_invite(&account.email, &contact.email, &body)
                    .await?;
                Ok((MessageStatus::Queued, None, None, None))
            }
            Some(their_keys) => {
                // Контакт активен — шифруем и отправляем
                let ciphertext = self
                    .encrypt_direct(
                        account_id, their_keys, conv_id, msg_id, &body, reply_to, &sent_at,
                    )
                    .await?;

                // Убеждаемся что папка EChat существует
                self.email.ensure_echat_folder().await.unwrap_or_else(|e| {
                    tracing::warn!("Не удалось создать папку EChat: {}", e);
                });

                // Отправляем через SMTP
                match self
                    .email
                    .send(OutgoingEmail {
                        from: account.email.clone(),
                        to: vec![contact.email.clone()],
                        subject: encryption::disguise::random_subject(),
                        body: ciphertext,
                        extra_headers: vec![("X-EChat".into(), "1".into())],
                    })
                    .await
                {
                    Ok(()) => Ok((MessageStatus::Sent, None, None, None)),
                    Err(e) => {
                        tracing::error!("Ошибка SMTP: {}", e);
                        Ok((MessageStatus::Failed, None, None, Some(e.to_string())))
                    }
                }
            }
        }
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

    /// Создаёт или возвращает существующую direct-беседу.
    ///
    /// При создании беседы генерирует пару ключей для этого диалога.
    pub async fn get_or_create_direct_conversation(
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

        // Генерируем пару ключей для этого диалога
        let keypair = encryption::keypair::IdentityKeypair::generate();
        
        // Сохраняем seed приватного ключа в base64
        let seed_base64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            keypair.seed().as_bytes(),
        );

        self.storage
            .create_conversation_keys(conv_id, seed_base64)
            .await?;

        self.storage.get_conversation(conv_id).await
    }

    // ── Ключи диалогов ───────────────────────────────────────────────────────

    /// Возвращает ключи диалога.
    pub async fn get_conversation_keys(&self, conv_id: Uuid) -> Result<ConversationKeys> {
        self.storage.get_conversation_keys(conv_id).await
    }

    /// Возвращает беседу по ID.
    pub async fn get_conversation(&self, conv_id: Uuid) -> Result<Conversation> {
        self.storage.get_conversation(conv_id).await
    }

    /// Устанавливает публичный ключ собеседника для диалога.
    pub async fn set_their_public_key(
        &self,
        conv_id: Uuid,
        their_public_key_base64: String,
    ) -> Result<()> {
        // their_public_key_base64 — это PublicKeys в base64 формате
        // Сохраняем как JSON (декодируем из base64)
        let their_keys = encryption::keypair::PublicKeys::from_base64(&their_public_key_base64)
            .map_err(|e| Error::Internal(format!("Ошибка парсинга ключа собеседника: {}", e)))?;

        let their_key_json = serde_json::to_string(&their_keys)
            .map_err(|e| Error::Internal(format!("Ошибка сериализации ключа: {}", e)))?;

        // Сохраняем ключ в conversation_keys (в JSON формате)
        self.storage
            .set_conversation_their_public_key(conv_id, their_key_json.clone())
            .await?;

        // Получаем беседу чтобы найти contact_id
        let conv = self.storage.get_conversation(conv_id).await?;

        // Если это direct беседа, обновляем статус контакта
        if let crate::models::conversation::ConversationKind::Direct { contact_id } = conv.kind {
            // Сохраняем публичный ключ в контакте (в JSON формате)
            self.storage
                .complete_contact_handshake(contact_id, their_key_json)
                .await?;
        }

        Ok(())
    }

    /// Импортирует seed нашего ключа для диалога (для восстановления истории).
    ///
    /// Принимает base64-encoded seed, из которого можно восстановить IdentityKeypair.
    /// Это позволяет расшифровать старые сообщения, если у пользователя есть backup ключей.
    pub async fn import_my_keypair_seed(&self, conv_id: Uuid, seed_base64: String) -> Result<()> {
        // Декодируем base64 seed
        let seed_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &seed_base64,
        )
        .map_err(|e| Error::Internal(format!("Ошибка декодирования base64 seed: {}", e)))?;

        // Проверяем размер
        let seed_array: [u8; 32] = seed_bytes
            .try_into()
            .map_err(|_| Error::Internal("Неверный размер seed: ожидается 32 байта".into()))?;

        // Пытаемся создать keypair из seed (проверка валидности)
        let keypair = encryption::keypair::IdentityKeypair::from_seed(
            encryption::keypair::KeySeed::from_bytes(seed_array),
        );

        // Получаем публичные ключи для отображения
        let public_keys = keypair.public_keys();
        let public_keys_json = serde_json::to_string(&public_keys)
            .map_err(|e| Error::Internal(format!("Ошибка сериализации публичных ключей: {}", e)))?;

        // Сохраняем seed в conversation_keys
        // Если ключи ещё не созданы, создаём их
        match self.storage.get_conversation_keys(conv_id).await {
            Ok(existing_keys) => {
                // Ключи существуют — обновляем my_keypair_json
                self.storage
                    .update_conversation_my_keypair(conv_id, seed_base64.clone())
                    .await?;

                // Если их ключ уже был установлен, обновляем его публичный ключ
                if let Some(their_key_json) = existing_keys.their_public_key_json {
                    // Обновляем публичный ключ контакта
                    if let Ok(conv) = self.storage.get_conversation(conv_id).await {
                        if let crate::models::conversation::ConversationKind::Direct { contact_id } =
                            conv.kind
                        {
                            let _ = self
                                .storage
                                .complete_contact_handshake(contact_id, their_key_json)
                                .await;
                        }
                    }
                }
            }
            Err(_) => {
                // Ключей нет — создаём новые
                self.storage
                    .create_conversation_keys(conv_id, seed_base64.clone())
                    .await?;
            }
        }

        tracing::info!(
            "Импортирован seed ключа для диалога {}. Публичный ключ: x25519={:x?}",
            conv_id,
            &public_keys.x25519[..4]
        );

        Ok(())
    }

    /// Проверяет, активны ли ключи диалога (оба ключа установлены).
    pub async fn are_keys_active(&self, conv_id: Uuid) -> Result<bool> {
        self.storage.are_conversation_keys_active(conv_id).await
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
        // if self.storage.message_exists(msg_id).await? {
        //     tracing::debug!("Дубликат сообщения {}, пропускаем", msg_id);
        //     return Ok(());
        // }

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
                error_message: None,
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

    /// Шифрует сообщение для direct-чата используя ключи диалога.
    async fn encrypt_direct_with_keys(
        &self,
        my_keypair_json: &str,
        their_keys: &encryption::keypair::PublicKeys,
        conv_id: Uuid,
        msg_id: Uuid,
        body: &str,
        reply_to: Option<Uuid>,
        sent_at: &chrono::DateTime<Utc>,
        _account_id: Uuid,
    ) -> Result<String> {
        // Используем ключи диалога для шифрования
        // my_keypair_json содержит seed в base64
        let seed_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            my_keypair_json,
        )
        .map_err(|e| Error::Internal(format!("Ошибка декодирования seed: {}", e)))?;

        let seed_array: [u8; 32] = seed_bytes
            .try_into()
            .map_err(|_| Error::Internal("Неверный размер seed".into()))?;

        let keypair = encryption::keypair::IdentityKeypair::from_seed(
            encryption::keypair::KeySeed::from_bytes(seed_array),
        );

        let shared_secret = encryption::session::derive_from_bytes(
            keypair.secret_key(),
            &their_keys.x25519,
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
