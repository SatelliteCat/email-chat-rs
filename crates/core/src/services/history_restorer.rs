//! HistoryRestorer — восстановление истории диалогов из писем в папке EChat.
//!
//! После авторизации обрабатывает все письма из папки EChat и:
//! - Создаёт диалоги при необходимости
//! - Декодирует и расшифровывает сообщения
//! - Исключает дубликаты по msg_id
//! - Определяет направление (кто кому писал) по отправителю/получателю
//!
//! ## Алгоритм
//!
//! ```text
//! 1. fetch_all_from_echat_folder() → Vec<IncomingEmail>
//! 2. Для каждого письма:
//!    a. Проверить X-EChat заголовок
//!    b. Извлечь payload
//!    c. Попробовать расшифровать
//!       - Если ключи есть → расшифровать → сохранить Message
//!       - Если ключей нет → сохранить как зашифрованный placeholder
//!    d. Создать/найти Conversation по email собеседника
//! 3. Обновить превью диалогов
//! ```

use std::collections::{HashMap, HashSet};

use chrono::Utc;
use uuid::Uuid;

use crate::{
    Error, Result,
    events::{ChatEvent, EventBus},
    models::{
        message::{Message, MessageKind, MessageStatus},
    },
    ports::{
        email::{DynEmailTransport, IncomingEmail},
        storage::{CreateMessage, DynStorage},
    },
    services::{account::AccountService, chat::ChatService},
};

/// Структура для хранения информации о собеседнике в контексте диалога.
struct PeerInfo {
    email: String,
    /// Направление: true = исходящее (мы писали), false = входящее
    is_outgoing: bool,
}

/// Восстановитель истории диалогов.
pub struct HistoryRestorer {
    storage: DynStorage,
    email_transport: DynEmailTransport,
    account_svc: AccountService,
    chat_svc: ChatService,
    events: EventBus,
}

impl HistoryRestorer {
    pub fn new(
        storage: DynStorage,
        email_transport: DynEmailTransport,
        account_svc: AccountService,
        chat_svc: ChatService,
        events: EventBus,
    ) -> Self {
        Self {
            storage,
            email_transport,
            account_svc,
            chat_svc,
            events,
        }
    }

    /// Восстанавливает историю диалогов из папки EChat.
    ///
    /// Возвращает количество обработанных сообщений и количество ошибок.
    pub async fn restore_history(&self, account_id: Uuid) -> Result<RestoreStats> {
        tracing::info!("Начало восстановления истории для аккаунта {}", account_id);

        let account = self.storage.get_account(account_id).await?;
        let my_email = account.email.clone();

        // Шаг 1: Получаем все письма из папки EChat
        let emails = match self.email_transport.restore_history(None).await {
            Ok(emails) => emails,
            Err(e) => {
                tracing::error!("Ошибка при получении писем из папки EChat: {}", e);
                return Err(Error::Internal(format!(
                    "Не удалось получить письма из папки EChat: {}",
                    e
                )));
            }
        };

        tracing::info!("Получено {} писем из папки EChat", emails.len());

        if emails.is_empty() {
            return Ok(RestoreStats {
                total_emails: 0,
                processed: 0,
                duplicates: 0,
                decrypted: 0,
                encrypted_stored: 0,
                errors: 0,
                conversations_created: 0,
            });
        }

        // Шаг 2: Обрабатываем каждое письмо
        let mut stats = RestoreStats {
            total_emails: emails.len(),
            processed: 0,
            duplicates: 0,
            decrypted: 0,
            encrypted_stored: 0,
            errors: 0,
            conversations_created: 0,
        };

        // Кэш для определения дубликатов msg_id
        let mut seen_msg_ids: HashSet<Uuid> = HashSet::new();

        // Кэш для сопоставления email → conversation_id
        let mut email_to_conv: HashMap<String, Uuid> = HashMap::new();

        for email in emails {
            match self
                .process_email(&mut stats, &mut seen_msg_ids, &mut email_to_conv, &my_email, email)
                .await
            {
                Ok(()) => {}
                Err(e) => {
                    tracing::warn!("Ошибка обработки письма: {}", e);
                    stats.errors += 1;
                }
            }
        }

        tracing::info!(
            "Восстановление завершено. Статистика: {:?}",
            stats
        );

        Ok(stats)
    }

    /// Обрабатывает одно письмо.
    async fn process_email(
        &self,
        stats: &mut RestoreStats,
        seen_msg_ids: &mut HashSet<Uuid>,
        email_to_conv: &mut HashMap<String, Uuid>,
        my_email: &str,
        email: IncomingEmail,
    ) -> Result<()> {
        // Шаг 1: Проверяем что это echat-письмо
        let headers: Vec<(&str, &str)> = email
            .headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        if !encryption::disguise::is_echat_message(&headers, Some(&email.body)) {
            tracing::debug!("Письмо uid={} не является echat-сообщением", email.uid);
            return Ok(());
        }

        stats.processed += 1;

        // Шаг 2: Извлекаем payload
        let raw = encryption::disguise::extract_payload(&email.body);
        let payload_single_line: String = raw.chars().filter(|c| !c.is_whitespace()).collect();

        // Шаг 3: Определяем собеседника и направление
        let peer_info = self.determine_peer(&email, my_email)?;

        // Шаг 4: Пытаемся расшифровать
        if encryption::cipher::EncryptedPayload::has_magic_prefix(&payload_single_line) {
            // Это зашифрованное сообщение
            match self
                .decrypt_and_process(
                    stats,
                    &peer_info.email,
                    &payload_single_line,
                    &peer_info,
                    my_email,
                    email_to_conv,
                    email.uid,
                    &email.folder,
                )
                .await
            {
                Ok(msg_id) => {
                    // Проверяем дубликат
                    if seen_msg_ids.contains(&msg_id) {
                        tracing::debug!("Дубликат сообщения {}, пропускаем", msg_id);
                        stats.duplicates += 1;
                        return Ok(());
                    }

                    seen_msg_ids.insert(msg_id);
                    stats.decrypted += 1;
                }
                Err(e) => {
                    tracing::warn!(
                        "Не удалось расшифровать сообщение uid={}: {}. Сохраняем как encrypted.",
                        email.uid,
                        e
                    );
                    // Сохраняем как зашифрованный placeholder
                    self.save_encrypted_placeholder(
                        stats,
                        &peer_info.email,
                        &payload_single_line,
                        &peer_info,
                        my_email,
                        email_to_conv,
                        email.uid,
                        &email.folder,
                    )
                    .await?;
                    stats.encrypted_stored += 1;
                }
            }
        } else {
            // Не распознано как зашифрованное — пропускаем
            tracing::debug!("Письмо uid={} не содержит зашифрованный payload", email.uid);
        }

        Ok(())
    }

    /// Определяет собеседника и направление письма.
    fn determine_peer(&self, email: &IncomingEmail, my_email: &str) -> Result<PeerInfo> {
        let from_lower = email.from.to_lowercase();
        let my_email_lower = my_email.to_lowercase();

        if from_lower == my_email_lower {
            // Мы отправили письмо — собеседник один из получателей
            let peer_email = email
                .to
                .iter()
                .find(|addr| addr.to_lowercase() != my_email_lower)
                .ok_or_else(|| {
                    Error::Internal(format!(
                        "Не удалось определить собеседника в исходящем письме от {}",
                        email.from
                    ))
                })?;

            Ok(PeerInfo {
                email: peer_email.to_string(),
                is_outgoing: true,
            })
        } else {
            // Нам отправили письмо — собеседник отправитель
            Ok(PeerInfo {
                email: from_lower.clone(),
                is_outgoing: false,
            })
        }
    }

    /// Расшифровывает сообщение и сохраняет в БД.
    async fn decrypt_and_process(
        &self,
        stats: &mut RestoreStats,
        peer_email: &str,
        payload_b64: &str,
        peer_info: &PeerInfo,
        my_email: &str,
        email_to_conv: &mut HashMap<String, Uuid>,
        imap_uid: u32,
        imap_folder: &str,
    ) -> Result<Uuid> {
        // Находим или создаём контакт
        let account = self
            .account_svc
            .storage()
            .get_account_by_email(my_email)
            .await?;

        let contact = match self
            .storage
            .get_contact_by_email(account.id, peer_email)
            .await
        {
            Ok(c) => c,
            Err(_) => {
                // Создаём контакт автоматически
                let contact_id = Uuid::new_v4();
                let create_contact = crate::ports::storage::CreateContact {
                    id: contact_id,
                    account_id: account.id,
                    name: peer_email.to_string(),
                    email: peer_email.to_string(),
                    avatar: None,
                };
                self.storage.create_contact(create_contact).await?;
                self.storage.get_contact(contact_id).await?
            }
        };

        // Находим или создаём беседу
        let conv_id = if let Some(&conv_id) = email_to_conv.get(peer_email) {
            conv_id
        } else {
            let conv = self
                .chat_svc
                .get_or_create_direct_conversation(account.id, contact.id)
                .await?;
            email_to_conv.insert(peer_email.to_string(), conv.id);
            stats.conversations_created += 1;
            conv.id
        };

        // Проверяем что ключи диалога активны
        let conv_keys = match self.storage.get_conversation_keys(conv_id).await {
            Ok(keys) => keys,
            Err(e) => {
                return Err(Error::Internal(format!(
                    "Ключи диалога {} не найдены: {}",
                    conv_id, e
                )));
            }
        };

        let my_keypair_json = conv_keys.my_keypair_json.ok_or_else(|| {
            Error::Internal("Наш keypair в диалоге не найден".into())
        })?;

        if my_keypair_json.starts_with('{') {
            return Err(Error::Internal(
                "Старый формат ключей диалога".into(),
            ));
        }

        let their_key_json = conv_keys.their_public_key_json.ok_or_else(|| {
            Error::Internal("Публичный ключ собеседника в диалоге не найден".into())
        })?;

        // Декодируем seed
        let seed_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &my_keypair_json,
        )
        .map_err(|e| Error::Internal(format!("Ошибка декодирования seed: {}", e)))?;

        let seed_array: [u8; 32] = seed_bytes
            .try_into()
            .map_err(|_| Error::Internal("Неверный размер seed".into()))?;

        let keypair = encryption::keypair::IdentityKeypair::from_seed(
            encryption::keypair::KeySeed::from_bytes(seed_array),
        );

        // Парсим публичные ключи собеседника
        let their_keys: encryption::keypair::PublicKeys =
            serde_json::from_str(&their_key_json).map_err(|e| {
                Error::Internal(format!("Ошибка парсинга ключей собеседника: {}", e))
            })?;

        // Вычисляем shared secret
        let shared_secret = encryption::session::derive_from_bytes(
            keypair.secret_key(),
            &their_keys.x25519,
            "direct-chat",
        )
        .map_err(|e| Error::Encryption(e.to_string()))?;

        // Декодируем и расшифровываем payload
        let payload = encryption::cipher::EncryptedPayload::from_base64(payload_b64)
            .map_err(|_| Error::Decrypt)?;

        let plaintext =
            encryption::cipher::decrypt(&payload, &shared_secret).map_err(|e| {
                Error::Internal(format!("Ошибка расшифровки: {}", e))
            })?;

        // Парсим ChatEnvelope
        let envelope: serde_json::Value =
            serde_json::from_slice(&plaintext).map_err(|e| {
                Error::Internal(format!("Ошибка парсинга envelope: {}", e))
            })?;

        let msg_id = envelope["msg_id"]
            .as_str()
            .ok_or_else(|| Error::Internal("msg_id не найден".into()))
            .and_then(|s| Uuid::parse_str(s).map_err(|e| Error::Internal(e.to_string())))?;

        let envelope_conv_id = envelope["conv_id"]
            .as_str()
            .ok_or_else(|| Error::Internal("conv_id не найден".into()))
            .and_then(|s| Uuid::parse_str(s).map_err(|e| Error::Internal(e.to_string())))?;

        // Проверяем что conv_id в envelope совпадает с нашим
        if envelope_conv_id != conv_id {
            tracing::warn!(
                "conv_id в envelope ({}) не совпадает с ожидаемым ({})",
                envelope_conv_id,
                conv_id
            );
        }

        let body = envelope["body"]
            .as_str()
            .ok_or_else(|| Error::Internal("body не найден".into()))
            .map(|s| s.to_string())?;

        let sent_at = envelope["sent_at"]
            .as_str()
            .ok_or_else(|| Error::Internal("sent_at не найден".into()))
            .and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(s)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .map_err(|e| Error::Internal(e.to_string()))
            })?;

        let reply_to = envelope["reply_to"]
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok());

        // Проверяем дубликат в БД
        if self.storage.message_exists(msg_id, Some(account.id)).await? {
            return Ok(msg_id); // Уже существует
        }

        // Определяем from_email в зависимости от направления
        let from_email = if peer_info.is_outgoing {
            my_email.to_string()
        } else {
            peer_email.to_string()
        };

        let now = Utc::now();
        let status = if peer_info.is_outgoing {
            MessageStatus::Sent
        } else {
            MessageStatus::Delivered
        };

        // Сохраняем сообщение
        self.storage
            .create_message(CreateMessage {
                id: msg_id,
                conversation_id: conv_id,
                account_id: account.id,
                from_email: from_email.clone(),
                body: Some(body.clone()),
                kind: MessageKind::Text,
                status: status.clone(),
                reply_to,
                imap_uid: Some(imap_uid),
                imap_folder: Some(imap_folder.to_string()),
                sent_at,
                error_message: None,
            })
            .await?;

        // Обновляем превью беседы
        self.storage
            .update_conversation_last_message(conv_id, body.clone(), sent_at, false)
            .await?;

        tracing::info!(
            "Восстановлено сообщение: msg_id={}, conv_id={}, from={}, sent_at={}",
            msg_id,
            conv_id,
            from_email,
            sent_at
        );

        // Эмитим событие
        let message = Message {
            id: msg_id,
            conversation_id: conv_id,
            account_id: account.id,
            from_email,
            body: Some(body),
            kind: MessageKind::Text,
            status,
            reply_to,
            imap_uid: Some(imap_uid),
            imap_folder: Some(imap_folder.to_string()),
            sent_at,
            received_at: Some(now),
        };

        self.events.emit(ChatEvent::NewMessage {
            conversation_id: conv_id,
            message,
        });

        Ok(msg_id)
    }

    /// Сохраняет зашифрованный placeholder когда не можем расшифровать.
    async fn save_encrypted_placeholder(
        &self,
        stats: &mut RestoreStats,
        peer_email: &str,
        _payload_b64: &str,
        peer_info: &PeerInfo,
        my_email: &str,
        email_to_conv: &mut HashMap<String, Uuid>,
        imap_uid: u32,
        imap_folder: &str,
    ) -> Result<()> {
        let account = self
            .account_svc
            .storage()
            .get_account_by_email(my_email)
            .await?;

        // Находим или создаём контакт
        let contact = match self
            .storage
            .get_contact_by_email(account.id, peer_email)
            .await
        {
            Ok(c) => c,
            Err(_) => {
                let contact_id = Uuid::new_v4();
                let create_contact = crate::ports::storage::CreateContact {
                    id: contact_id,
                    account_id: account.id,
                    name: peer_email.to_string(),
                    email: peer_email.to_string(),
                    avatar: None,
                };
                self.storage.create_contact(create_contact).await?;
                self.storage.get_contact(contact_id).await?
            }
        };

        // Находим или создаём беседу
        let conv_id = if let Some(&conv_id) = email_to_conv.get(peer_email) {
            conv_id
        } else {
            let conv = self
                .chat_svc
                .get_or_create_direct_conversation(account.id, contact.id)
                .await?;
            email_to_conv.insert(peer_email.to_string(), conv.id);
            stats.conversations_created += 1;
            conv.id
        };

        // Генерируем placeholder msg_id на основе imap_uid чтобы избежать дубликатов
        // Используем простой хеш: берём UID и folder как основу для Uuid
        let placeholder_msg_id = {
            let hash_str = format!("{}-{}", imap_folder, imap_uid);
            let hash_bytes = hash_str.as_bytes();
            let mut bytes = [0u8; 16];
            for (i, &b) in hash_bytes.iter().take(16).enumerate() {
                bytes[i] = b;
            }
            Uuid::from_bytes(bytes)
        };

        // Проверяем дубликат
        if self
            .storage
            .message_exists(placeholder_msg_id, Some(account.id))
            .await?
        {
            return Ok(());
        }

        let from_email = if peer_info.is_outgoing {
            my_email.to_string()
        } else {
            peer_email.to_string()
        };

        let now = Utc::now();

        // Сохраняем как placeholder с зашифрованным телом
        self.storage
            .create_message(CreateMessage {
                id: placeholder_msg_id,
                conversation_id: conv_id,
                account_id: account.id,
                from_email: from_email.clone(),
                body: Some("[зашифровано — ключи не найдены]".to_string()),
                kind: MessageKind::Text,
                status: MessageStatus::Delivered,
                reply_to: None,
                imap_uid: Some(imap_uid),
                imap_folder: Some(imap_folder.to_string()),
                sent_at: now,
                error_message: None,
            })
            .await?;

        tracing::info!(
            "Сохранён зашифрованный placeholder: conv_id={}, imap_uid={}",
            conv_id,
            imap_uid
        );

        Ok(())
    }
}

/// Статистика восстановления.
#[derive(Debug, Clone, Default)]
pub struct RestoreStats {
    /// Всего писем получено из папки
    pub total_emails: usize,
    /// Писем обработано (echat-сообщения)
    pub processed: usize,
    /// Дубликатов пропущено
    pub duplicates: usize,
    /// Сообщений расшифровано
    pub decrypted: usize,
    /// Зашифрованных сохранено как placeholder
    pub encrypted_stored: usize,
    /// Ошибок при обработке
    pub errors: usize,
    /// Диалогов создано
    pub conversations_created: usize,
}
