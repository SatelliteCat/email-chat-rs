use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    pub id: Uuid,
    pub account_id: Uuid,
    pub name: String,
    pub email: String,
    pub avatar: Option<Vec<u8>>,
    /// Статус контакта — показывает, есть ли публичный ключ для шифрования.
    pub status: ContactStatus,
    /// Публичные ключи (X25519 + Ed25519) после завершения handshake.
    pub public_keys: Option<ContactPublicKeys>,
    pub handshake_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Статус контакта — показывает наличие публичного ключа.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContactStatus {
    /// Публичного ключа нет — шифрование невозможно
    NoKey,
    /// Публичный ключ сохранён — можно шифровать
    HasKey,
}

impl std::fmt::Display for ContactStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContactStatus::NoKey => write!(f, "Нет ключа"),
            ContactStatus::HasKey => write!(f, "Ключ есть"),
        }
    }
}

/// Публичные ключи контакта — после успешного handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactPublicKeys {
    /// X25519 pubkey для DH (32 байта)
    pub x25519: [u8; 32],
    /// Ed25519 pubkey для верификации подписей (32 байта)
    pub ed25519: [u8; 32],
}

impl ContactPublicKeys {
    pub fn x25519_bytes(&self) -> Option<[u8; 32]> {
        Some(self.x25519)
    }

    pub fn ed25519_bytes(&self) -> Option<[u8; 32]> {
        Some(self.ed25519)
    }
}
