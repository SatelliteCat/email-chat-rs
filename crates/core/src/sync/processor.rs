//! Processor — разбирает входящее письмо и направляет в нужный сервис.
//!
//! ```text
//! IncomingEmail
//!     │
//!     ├─ is_echat_message? ──── нет ──► игнорируем
//!     │
//!     └─ да
//!         │
//!         ├─ HandshakeInit  ──► ContactService::handle_handshake_init
//!         ├─ HandshakeAck   ──► ContactService::handle_handshake_ack
//!         └─ EncryptedMsg   ──► расшифровать → ChatService::handle_incoming
//! ```

use uuid::Uuid;

use crate::{
    Error, Result,
    ports::email::IncomingEmail,
    services::{account::AccountService, chat::ChatService, contacts::ContactService},
};

/// Тип входящего echat-письма после первичного разбора.
#[derive(Debug)]
enum IncomingKind {
    HandshakeInit(encryption::keypair::PublicKeys),
    HandshakeAck(encryption::keypair::PublicKeys),
    EncryptedMessage { payload_b64: String },
    Unknown,
}

/// Обрабатывает одно входящее письмо.
///
/// Возвращает Ok(()) даже если письмо не является echat-сообщением.
pub async fn process_incoming(
    email: &IncomingEmail,
    account_id: Uuid,
    account_svc: &AccountService,
    contact_svc: &ContactService,
    chat_svc: &ChatService,
) -> Result<()> {
    // Шаг 1: проверяем заголовок X-EChat
    let headers: Vec<(&str, &str)> = email
        .headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    if !encryption::disguise::is_echat_message(&headers, Some(&email.body)) {
        return Ok(()); // обычное письмо — пропускаем
    }

    // Шаг 2: извлекаем payload
    let raw = encryption::disguise::extract_payload(&email.body);

    // Шаг 3: определяем тип
    let kind = detect_kind(raw);

    // Шаг 4: направляем в нужный сервис
    match kind {
        IncomingKind::HandshakeInit(their_keys) => {
            tracing::info!("HandshakeInit от {}", email.from);
            contact_svc
                .handle_handshake_init(account_id, &email.from, &their_keys)
                .await?;
        }

        IncomingKind::HandshakeAck(their_keys) => {
            tracing::info!("HandshakeAck от {}", email.from);
            contact_svc
                .handle_handshake_ack(account_id, &email.from, &their_keys)
                .await?;
        }

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
fn detect_kind(raw: &str) -> IncomingKind {
    // Пробуем как HandshakeMessage
    if let Ok(hs) = encryption::handshake::HandshakeMessage::from_base64(raw) {
        if hs.verify(3600).is_ok() {
            return match hs.kind {
                encryption::handshake::HandshakeKind::Init => {
                    IncomingKind::HandshakeInit(hs.public_keys)
                }
                encryption::handshake::HandshakeKind::Ack => {
                    IncomingKind::HandshakeAck(hs.public_keys)
                }
            };
        }
    }

    // Пробуем как EncryptedPayload (по magic bytes)
    if encryption::cipher::EncryptedPayload::has_magic_prefix(raw) {
        return IncomingKind::EncryptedMessage {
            payload_b64: raw.to_string(),
        };
    }

    IncomingKind::Unknown
}

/// Расшифровывает зашифрованное сообщение.
async fn decrypt_message(
    _account_id: Uuid,
    _from_email: &str,
    _payload_b64: &str,
    _account_svc: &AccountService,
) -> Result<(Uuid, Uuid, String, chrono::DateTime<chrono::Utc>)> {
    // TODO: нужен доступ к StoragePort для получения публичного ключа контакта
    // Пока оставляем как заглушку — будет реализовано при интеграции
    Err(Error::Internal(
        "decrypt_message: требует StoragePort (TODO)".into(),
    ))
}
