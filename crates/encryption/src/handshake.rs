//! Протокол обмена публичными ключами.
//!
//! ## Последовательность
//!
//! ```text
//! Alice                                      Bob
//!   |                                          |
//!   |-- HandshakeInit -----------------------> |
//!   |   { public_keys, signature, timestamp }  |
//!   |                                          |
//!   | <----------------------- HandshakeAck -- |
//!   |         { public_keys, signature }       |
//!   |                                          |
//!   |  Оба вычисляют SharedSecret              |
//!   |  и могут обмениваться сообщениями        |
//! ```
//!
//! Подпись покрывает публичные ключи + timestamp, что предотвращает
//! replay-атаки и подмену ключей.

use serde::{Deserialize, Serialize};
use serde_with::{Bytes, serde_as};

use crate::{
    Error, Result,
    keypair::{IdentityKeypair, PublicKeys},
};

/// Тип handshake сообщения.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandshakeKind {
    /// Инициатор начинает обмен
    Init,
    /// Ответ на Init
    Ack,
}

/// Handshake-сообщение — кладётся в тело письма вместо EncryptedPayload.
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeMessage {
    /// Версия протокола
    pub version: u8,
    /// Тип: Init или Ack
    pub kind: HandshakeKind,
    /// Публичные ключи отправителя
    pub public_keys: PublicKeys,
    /// Ed25519 подпись (покрывает public_keys + timestamp_secs)
    #[serde_as(as = "Bytes")]
    pub signature: [u8; 64],
    /// Unix timestamp в секундах (для защиты от replay)
    pub timestamp_secs: u64,
    /// Email отправителя (для верификации при получении)
    pub from_email: String,
}

impl HandshakeMessage {
    /// Создаёт Init-сообщение.
    pub fn new_init(keypair: &IdentityKeypair, from_email: &str) -> Self {
        let timestamp_secs = current_timestamp();
        let public_keys = keypair.public_keys();
        let signature = sign_handshake(keypair, &public_keys, timestamp_secs);

        Self {
            version: 1,
            kind: HandshakeKind::Init,
            public_keys,
            signature,
            timestamp_secs,
            from_email: from_email.to_string(),
        }
    }

    /// Создаёт Ack-сообщение в ответ на полученный Init.
    pub fn new_ack(keypair: &IdentityKeypair, from_email: &str) -> Self {
        let timestamp_secs = current_timestamp();
        let public_keys = keypair.public_keys();
        let signature = sign_handshake(keypair, &public_keys, timestamp_secs);

        Self {
            version: 1,
            kind: HandshakeKind::Ack,
            public_keys,
            signature,
            timestamp_secs,
            from_email: from_email.to_string(),
        }
    }

    /// Проверяет подпись и актуальность timestamp.
    ///
    /// `max_age_secs` — максимальный допустимый возраст сообщения (обычно 3600 секунд).
    pub fn verify(&self, max_age_secs: u64) -> Result<()> {
        // Проверяем актуальность
        let now = current_timestamp();
        let age = now.saturating_sub(self.timestamp_secs);
        if age > max_age_secs {
            return Err(Error::InvalidSignature);
        }

        // Проверяем подпись
        let data = signing_data(&self.public_keys, self.timestamp_secs);
        IdentityKeypair::verify(&self.public_keys.ed25519, &data, &self.signature)
    }

    /// Кодирует в base64 для вставки в тело письма.
    pub fn to_base64(&self) -> Result<String> {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let bytes = serde_json::to_vec(self)?;
        Ok(STANDARD.encode(bytes))
    }

    /// Декодирует из base64.
    pub fn from_base64(s: &str) -> Result<Self> {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let bytes = STANDARD
            .decode(s.trim())
            .map_err(|_| Error::InvalidPublicKey("base64 decode failed".into()))?;
        serde_json::from_slice(&bytes).map_err(Error::Serialization)
    }
}

/// Состояние handshake с конкретным контактом.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandshakeState {
    /// Исходящий Init отправлен, ждём Ack
    WaitingForAck,
    /// Входящий Init получен, отправляем Ack
    AckSent,
    /// Завершён успешно, shared secret установлен
    Complete,
}

// ── Внутренние утилиты ────────────────────────────────────────────────────────

fn signing_data(public_keys: &PublicKeys, timestamp_secs: u64) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&public_keys.x25519);
    data.extend_from_slice(&public_keys.ed25519);
    data.extend_from_slice(&timestamp_secs.to_le_bytes());
    data
}

fn sign_handshake(
    keypair: &IdentityKeypair,
    public_keys: &PublicKeys,
    timestamp_secs: u64,
) -> [u8; 64] {
    let data = signing_data(public_keys, timestamp_secs);
    keypair.sign(&data)
}

fn current_timestamp() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_verify() {
        let alice = IdentityKeypair::generate();
        let msg = HandshakeMessage::new_init(&alice, "alice@mail.ru");
        assert!(msg.verify(3600).is_ok());
    }

    #[test]
    fn test_tampered_message_fails() {
        let alice = IdentityKeypair::generate();
        let mut msg = HandshakeMessage::new_init(&alice, "alice@mail.ru");

        // Портим публичный ключ
        msg.public_keys.x25519[0] ^= 0xFF;

        assert!(msg.verify(3600).is_err());
    }

    #[test]
    fn test_base64_roundtrip() {
        let alice = IdentityKeypair::generate();
        let msg = HandshakeMessage::new_ack(&alice, "alice@yandex.ru");

        let encoded = msg.to_base64().unwrap();
        let decoded = HandshakeMessage::from_base64(&encoded).unwrap();

        assert_eq!(msg.public_keys, decoded.public_keys);
        assert_eq!(msg.kind, decoded.kind);
        assert_eq!(msg.from_email, decoded.from_email);
    }
}
