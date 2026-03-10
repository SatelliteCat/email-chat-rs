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
    pub status: ContactStatus,
    /// Публичные ключи (X25519 + Ed25519) после завершения handshake.
    pub public_keys: Option<ContactPublicKeys>,
    pub handshake_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContactStatus {
    /// Приложение не установлено — отправлено приглашение
    Unregistered,
    /// Handshake отправлен, ждём ответа
    Pending,
    /// Канал установлен, шифруем
    Active,
}

impl std::fmt::Display for ContactStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContactStatus::Unregistered => write!(f, "Не зарегистрирован"),
            ContactStatus::Pending => write!(f, "Ожидание"),
            ContactStatus::Active => write!(f, "Активен"),
        }
    }
}

/// Публичные ключи контакта — после успешного handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactPublicKeys {
    /// X25519 pubkey для DH (32 байта, hex)
    pub x25519: String,
    /// Ed25519 pubkey для верификации подписей (32 байта, hex)
    pub ed25519: String,
}

impl ContactPublicKeys {
    pub fn x25519_bytes(&self) -> Option<[u8; 32]> {
        let bytes = hex::decode(&self.x25519).ok()?;
        bytes.try_into().ok()
    }

    pub fn ed25519_bytes(&self) -> Option<[u8; 32]> {
        let bytes = hex::decode(&self.ed25519).ok()?;
        bytes.try_into().ok()
    }
}
