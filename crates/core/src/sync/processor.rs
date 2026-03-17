//! Processor — разбирает входящее письмо и направляет в нужный сервис.
//!
//! ```text
//! IncomingEmail
//!     │
//!     ├─ is_echat_message? ──── нет ──► игнорируем
//!     │
//!     └─ да
//!         │
//!         └─ EncryptedMsg ──► расшифровать → ChatService::handle_incoming
//! ```
//!
//! Handshake не реализован — пользователь вручную вводит публичные ключи через UI.

use uuid::Uuid;

use crate::{
    Error, Result,
    ports::email::{DynEmailTransport, IncomingEmail},
    services::{account::AccountService, chat::ChatService},
};

/// Тип входящего echat-письма после первичного разбора.
#[derive(Debug)]
enum IncomingKind {
    EncryptedMessage { payload_b64: String },
    Unknown,
}

/// Обрабатывает одно входящее письмо.
///
/// Возвращает Ok(()) даже если письмо не является echat-сообщением.
pub async fn process_incoming(
    email: &IncomingEmail,
    account_id: Uuid,
    email_transport: &DynEmailTransport,
    account_svc: &AccountService,
    chat_svc: &ChatService,
) -> Result<()> {
    tracing::debug!(
        "Обработка письма от {}, UID: {}, folder: {}",
        email.from,
        email.uid,
        email.folder
    );

    // Шаг 1: проверяем заголовок X-EChat
    let headers: Vec<(&str, &str)> = email
        .headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    tracing::debug!("Заголовки письма: {:?}", headers);

    if !encryption::disguise::is_echat_message(&headers, Some(&email.body)) {
        tracing::debug!("Письмо от {} не является echat-сообщением", email.from);
        tracing::debug!(
            "Тело письма (первые 200 символов): {}",
            email.body.chars().take(200).collect::<String>()
        );
        return Ok(()); // обычное письмо — пропускаем
    }

    tracing::info!("Получено echat-письмо от {}", email.from);

    // Шаг 2: перемещаем письмо в папку EChat если оно было в INBOX
    let mut email = email.clone();
    if email.folder == "INBOX" {
        if let Ok(account) = account_svc.storage().get_account(account_id).await {
            let target_folder = &account.echat_folder;
            if target_folder != "INBOX" {
                tracing::debug!("Перемещаем письмо из INBOX в {}", target_folder);
                match email_transport
                    .move_messages("INBOX", target_folder, &[email.uid])
                    .await
                {
                    Ok(()) => {
                        email.folder = target_folder.clone();
                        tracing::info!("Письмо перемещено в {}", target_folder);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Не удалось переместить письмо в {}: {}. Продолжаем обработку.",
                            target_folder,
                            e
                        );
                    }
                }
            }
        }
    }

    // Шаг 3: извлекаем payload и удаляем переносы строк
    let raw = encryption::disguise::extract_payload(&email.body);
    let raw_single_line: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    tracing::debug!(
        "Payload (первые 100 символов): {}",
        raw_single_line.chars().take(100).collect::<String>()
    );

    // Шаг 4: определяем тип и направляем в сервис
    let kind = detect_kind(&raw_single_line, &email.from);

    match kind {
        IncomingKind::EncryptedMessage { payload_b64 } => {
            // Пытаемся расшифровать
            match decrypt_message(account_id, &email.from, &payload_b64, account_svc).await {
                Ok((conv_id, msg_id, body, sent_at)) => {
                    chat_svc
                        .handle_incoming(
                            account_id,
                            email.from.clone(),
                            conv_id,
                            msg_id,
                            body,
                            sent_at,
                            email.uid,
                            email.folder.clone(),
                        )
                        .await?;
                }
                Err(e) => {
                    tracing::warn!("Не удалось расшифровать сообщение от {}: {}", email.from, e);
                    // Не возвращаем ошибку — письмо могло быть от другого ключа
                }
            }
        }

        IncomingKind::Unknown => {
            tracing::debug!("Неизвестный тип echat-письма от {}", email.from);
        }
    }

    Ok(())
}

/// Определяет тип входящего payload.
fn detect_kind(raw: &str, from_email: &str) -> IncomingKind {
    tracing::debug!(
        "Определяем тип payload от {}, длина: {} символов",
        from_email,
        raw.len()
    );

    // Пробуем как EncryptedPayload (по magic bytes)
    if encryption::cipher::EncryptedPayload::has_magic_prefix(raw) {
        tracing::debug!(
            "Обнаружен EncryptedPayload от {} по magic bytes",
            from_email
        );
        return IncomingKind::EncryptedMessage {
            payload_b64: raw.to_string(),
        };
    }

    tracing::debug!("Неизвестный тип payload от {}", from_email);
    IncomingKind::Unknown
}

/// Расшифровывает зашифрованное сообщение.
async fn decrypt_message(
    account_id: Uuid,
    from_email: &str,
    payload_b64: &str,
    account_svc: &AccountService,
) -> Result<(Uuid, Uuid, String, chrono::DateTime<chrono::Utc>)> {
    // Загружаем наш keypair
    let keypair = account_svc.load_or_create_keypair(account_id).await?;

    // Находим контакт по email чтобы получить его публичные ключи
    // Используем storage напрямую через trait object
    let storage = account_svc.storage();
    let contact = storage
        .get_contact_by_email(account_id, from_email)
        .await
        .map_err(|_| Error::NotFound(format!("Контакт {} не найден", from_email)))?;

    // Получаем публичные ключи контакта
    // Пробуем сначала из контакта, потом из conversation_keys
    let their_keys = if let Some(keys) = &contact.public_keys {
        keys.clone()
    } else {
        // Пытаемся найти ключи в conversation_keys
        tracing::debug!("У контакта {} нет public_keys, ищем в conversation_keys", from_email);
        
        // Находим беседу
        if let Some(conv) = storage
            .find_direct_conversation(account_id, contact.id)
            .await?
        {
            // Получаем ключи беседы
            if let Ok(conv_keys) = storage.get_conversation_keys(conv.id).await {
                if let Some(their_key_json) = conv_keys.their_public_key_json {
                    tracing::debug!("Найдены ключи в conversation_keys для беседы {}", conv.id);
                    serde_json::from_str(&their_key_json)
                        .map_err(|e| Error::Internal(format!("Ошибка парсинга ключей: {}", e)))?
                } else {
                    return Err(Error::Encryption(
                        "У контакта нет публичных ключей (в conversation_keys их тоже нет)".into()
                    ));
                }
            } else {
                return Err(Error::Encryption(
                    "У контакта нет публичных ключей (conversation_keys не найдены)".into()
                ));
            }
        } else {
            return Err(Error::Encryption(
                "У контакта нет публичных ключей (беседа не найдена)".into()
            ));
        }
    };

    // Вычисляем shared secret
    let their_x25519 = their_keys
        .x25519_bytes()
        .ok_or_else(|| Error::Encryption("Некорректный X25519 ключ контакта".into()))?;

    let shared_secret =
        encryption::session::derive_from_bytes(keypair.secret_key(), &their_x25519, "direct-chat")
            .map_err(|e| Error::Encryption(e.to_string()))?;

    // Декодируем и расшифровываем payload
    let payload = encryption::cipher::EncryptedPayload::from_base64(payload_b64)
        .map_err(|_| Error::Decrypt)?;

    let plaintext =
        encryption::cipher::decrypt(&payload, &shared_secret).map_err(|_| Error::Decrypt)?;

    // Парсим ChatEnvelope из plaintext
    let envelope: serde_json::Value =
        serde_json::from_slice(&plaintext).map_err(|e| Error::Internal(e.to_string()))?;

    let msg_id = envelope["msg_id"]
        .as_str()
        .ok_or_else(|| Error::Internal("msg_id не найден".into()))
        .and_then(|s| Uuid::parse_str(s).map_err(|e| Error::Internal(e.to_string())))?;

    let conv_id = envelope["conv_id"]
        .as_str()
        .ok_or_else(|| Error::Internal("conv_id не найден".into()))
        .and_then(|s| Uuid::parse_str(s).map_err(|e| Error::Internal(e.to_string())))?;

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

    Ok((conv_id, msg_id, body, sent_at))
}
