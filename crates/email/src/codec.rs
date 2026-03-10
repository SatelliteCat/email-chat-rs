//! Конвертация между raw email и структурами приложения.
//!
//! Этот модуль — мост между email-протоколом и логикой чата:
//!
//! ```text
//! OutgoingMessage  ←──  encode()  ←──  ChatEnvelope
//!                                            │
//!                                    encrypt (crates/encryption)
//!
//! IncomingMessage  ──►  decode()  ──►  ChatEnvelope
//!                                            │
//!                                    decrypt (crates/encryption)
//! ```
//!
//! Codec знает о disguise (маскировка), но НЕ знает об encryption —
//! шифрование/расшифровка происходит выше, в core/services.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use encryption::disguise::{self, BodyKind};

use crate::{
    types::{IncomingMessage, OutgoingMessage},
    Error, Result,
};

/// Тип содержимого envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnvelopeKind {
    /// Обычное зашифрованное сообщение (direct или группа)
    Message,
    /// Handshake: обмен публичными ключами
    Handshake,
    /// Handshake Ack
    HandshakeAck,
    /// Приглашение пользователю без приложения
    Invite,
}

/// Расшифрованный envelope — то что живёт внутри зашифрованного payload.
/// Сериализуется в JSON перед шифрованием.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatEnvelope {
    /// Уникальный ID сообщения
    pub msg_id: Uuid,
    /// ID беседы (для группировки в чат)
    pub conv_id: Uuid,
    /// Тип
    pub kind: EnvelopeKind,
    /// Время отправки (UTC)
    pub sent_at: DateTime<Utc>,
    /// Текст сообщения (для kind = Message)
    pub body: Option<String>,
    /// На какое сообщение отвечает (reply)
    pub reply_to: Option<Uuid>,
    /// Версия протокола
    pub protocol_version: u8,
}

impl ChatEnvelope {
    /// Создаёт новый envelope для текстового сообщения.
    pub fn new_message(conv_id: Uuid, body: String, reply_to: Option<Uuid>) -> Self {
        Self {
            msg_id: Uuid::new_v4(),
            conv_id,
            kind: EnvelopeKind::Message,
            sent_at: Utc::now(),
            body: Some(body),
            reply_to,
            protocol_version: 1,
        }
    }

    /// Создаёт envelope для handshake.
    pub fn new_handshake(conv_id: Uuid) -> Self {
        Self {
            msg_id: Uuid::new_v4(),
            conv_id,
            kind: EnvelopeKind::Handshake,
            sent_at: Utc::now(),
            body: None,
            reply_to: None,
            protocol_version: 1,
        }
    }

    /// Сериализует в байты для шифрования.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(|e| Error::Codec(e.to_string()))
    }

    /// Десериализует из байт после расшифровки.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes).map_err(|e| Error::Codec(e.to_string()))
    }
}

/// Подготавливает OutgoingMessage для отправки зашифрованного сообщения.
///
/// `payload_base64` — уже зашифрованный и закодированный в base64 payload
/// (из `encryption::cipher::encrypt` или `encryption::group::encrypt`).
pub fn encode_message(
    from: &str,
    to: &[String],
    payload_base64: &str,
) -> OutgoingMessage {
    let email = disguise::build_email(payload_base64, BodyKind::EncryptedMessage);
    OutgoingMessage {
        from: from.to_string(),
        to: to.to_vec(),
        subject: email.subject,
        body: email.body,
        extra_headers: email.extra_headers,
    }
}

/// Подготавливает OutgoingMessage для handshake.
pub fn encode_handshake(
    from: &str,
    to: &str,
    handshake_base64: &str,
    is_ack: bool,
) -> OutgoingMessage {
    let kind = if is_ack {
        BodyKind::Handshake // Ack выглядит как обычное зашифрованное
    } else {
        BodyKind::Handshake
    };
    let email = disguise::build_email(handshake_base64, kind);
    OutgoingMessage {
        from: from.to_string(),
        to: vec![to.to_string()],
        subject: email.subject,
        body: email.body,
        extra_headers: email.extra_headers,
    }
}

/// Подготавливает OutgoingMessage для приглашения (контакт без приложения).
pub fn encode_invite(
    from: &str,
    to: &str,
    payload_base64: &str,
    app_url: &str,
) -> OutgoingMessage {
    let email = disguise::build_email(
        payload_base64,
        BodyKind::Invite {
            app_url: app_url.to_string(),
        },
    );
    OutgoingMessage {
        from: from.to_string(),
        to: vec![to.to_string()],
        subject: email.subject,
        body: email.body,
        extra_headers: email.extra_headers,
    }
}

/// Проверяет входящее письмо — является ли оно echat-сообщением.
pub fn is_echat_message(msg: &IncomingMessage) -> bool {
    let pairs = msg.headers.as_str_pairs();
    disguise::is_echat_message(&pairs, Some(&msg.body))
}

/// Извлекает payload base64 из тела входящего письма.
pub fn extract_payload(msg: &IncomingMessage) -> &str {
    disguise::extract_payload(&msg.body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_envelope_roundtrip() {
        let conv_id = Uuid::new_v4();
        let env = ChatEnvelope::new_message(
            conv_id,
            "Hello!".to_string(),
            None,
        );

        let bytes = env.to_bytes().unwrap();
        let decoded = ChatEnvelope::from_bytes(&bytes).unwrap();

        assert_eq!(env.msg_id, decoded.msg_id);
        assert_eq!(env.conv_id, decoded.conv_id);
        assert_eq!(env.body, decoded.body);
    }

    #[test]
    fn test_encode_message_has_echat_header() {
        let msg = encode_message(
            "alice@mail.ru",
            &["bob@yandex.ru".to_string()],
            "dGVzdA==",
        );
        assert!(msg
            .extra_headers
            .iter()
            .any(|(k, v)| k == "X-EChat" && v == "1"));
    }

    #[test]
    fn test_encode_invite_has_app_url() {
        let msg = encode_invite(
            "alice@mail.ru",
            "bob@example.com",
            "dGVzdA==",
            "https://echat.app",
        );
        assert!(msg.body.contains("https://echat.app"));
        assert!(msg.body.contains("dGVzdA=="));
    }
}
