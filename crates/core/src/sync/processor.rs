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

    // tracing::debug!("Заголовки письма: {:?}", headers);

    if !encryption::disguise::is_echat_message(&headers, Some(&email.body)) {
        tracing::debug!("Письмо от {} не является echat-сообщением", email.from);
        tracing::debug!(
            "Тело письма (первые 50 символов): {}",
            email.body.chars().take(50).collect::<String>()
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
                // Убеждаемся что папка существует
                tracing::debug!("Проверяем/создаём папку {}", target_folder);
                if let Err(e) = email_transport.ensure_echat_folder().await {
                    tracing::warn!("Не удалось создать папку {}: {}", target_folder, e);
                }
                
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
        "Payload (первые 50 символов): {}",
        raw_single_line.chars().take(50).collect::<String>()
    );

    // Шаг 4: определяем тип и направляем в сервис
    let kind = detect_kind(&raw_single_line, &email.from);

    match kind {
        IncomingKind::EncryptedMessage { payload_b64 } => {
            // Пытаемся расшифровать
            match decrypt_message(account_id, &email.from, &payload_b64, account_svc).await {
                Ok((conv_id, msg_id, body, sent_at)) => {
                    tracing::info!("Вызов handle_incoming для msg_id={}", msg_id);
                    match chat_svc
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
                        .await
                    {
                        Ok(()) => tracing::info!("handle_incoming успешен для msg_id={}", msg_id),
                        Err(e) => {
                            tracing::error!("handle_incoming ошибка для msg_id={}: {}", msg_id, e);
                            return Err(e);
                        }
                    }
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
    let storage = account_svc.storage();

    // Находим контакт по email
    let contact = storage
        .get_contact_by_email(account_id, from_email)
        .await
        .map_err(|_| Error::NotFound(format!("Контакт {} не найден", from_email)))?;

    // Находим беседу
    let conv = storage
        .find_direct_conversation(account_id, contact.id)
        .await?
        .ok_or_else(|| Error::NotFound("Беседа не найдена".into()))?;

    // Получаем ключи диалога
    let conv_keys = storage
        .get_conversation_keys(conv.id)
        .await
        .map_err(|e| Error::Encryption(format!("Ключи диалога не найдены: {}", e)))?;

    // Получаем наш keypair из ключей диалога
    // my_keypair_json может быть:
    // - base64 seed (новый формат)
    // - JSON публичных ключей (старый формат, для обратной совместимости)
    let my_keypair_json = conv_keys
        .my_keypair_json
        .ok_or_else(|| Error::Encryption("Наш keypair в диалоге не найден".into()))?;

    let keypair = if my_keypair_json.starts_with('{') {
        // Старый формат: JSON публичных ключей — ошибка, нужны пересоздать беседу
        tracing::warn!("Обнаружен старый формат ключей диалога (JSON)");
        return Err(Error::Encryption(
            "Старый формат ключей диалога. Пересоздайте беседу.".into()
        ));
    } else {
        // Новый формат: base64 seed
        let seed_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &my_keypair_json,
        )
        .map_err(|e| Error::Internal(format!("Ошибка декодирования seed: {}", e)))?;

        let seed_array: [u8; 32] = seed_bytes
            .try_into()
            .map_err(|_| Error::Internal("Неверный размер seed".into()))?;

        encryption::keypair::IdentityKeypair::from_seed(
            encryption::keypair::KeySeed::from_bytes(seed_array),
        )
    };

    // Получаем публичный ключ собеседника из ключей диалога
    // their_public_key_json хранится как JSON PublicKeys
    let their_key_json = conv_keys
        .their_public_key_json
        .ok_or_else(|| Error::Encryption("Публичный ключ собеседника в диалоге не найден".into()))?;

    tracing::debug!("their_key_json (первые 50): {:?}", their_key_json.chars().take(50).collect::<String>());

    // Парсим JSON PublicKeys
    let their_keys: encryption::keypair::PublicKeys =
        serde_json::from_str(&their_key_json)
            .map_err(|e| {
                tracing::error!("Ошибка парсинга their_key_json={:?}: {}", their_key_json, e);
                Error::Internal(format!("Ошибка парсинга ключей собеседника: {}", e))
            })?;

    // Вычисляем shared secret используя ключи диалога
    let shared_secret =
        encryption::session::derive_from_bytes(keypair.secret_key(), &their_keys.x25519, "direct-chat")
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

    tracing::info!("Расшифровано сообщение: msg_id={}, conv_id={}, from={}", msg_id, conv_id, from_email);

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
