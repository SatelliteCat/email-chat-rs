//! # encryption
//!
//! E2E шифрование для email-chat.
//!
//! ## Модули
//!
//! - [`keypair`]   — генерация и хранение ключей (X25519 + Ed25519)
//! - [`session`]   — Diffie-Hellman, вычисление общего секрета
//! - [`cipher`]    — шифрование/расшифрование сообщений (ChaCha20-Poly1305)
//! - [`handshake`] — протокол обмена публичными ключами
//! - [`group`]     — fan-out шифрование для групповых чатов
//! - [`export`]    — экспорт/импорт приватного ключа
//! - [`disguise`]  — маскировка email-заголовков
//!
//! ## Пример использования (direct чат)
//!
//! ```rust
//! use encryption::prelude::*;
//!
//! // Alice и Bob генерируют ключи
//! let alice = IdentityKeypair::generate();
//! let bob   = IdentityKeypair::generate();
//!
//! // Вычисляют общий секрет (X25519 DH) с контекстом
//! let alice_shared = session::derive_shared_secret(
//!     alice.secret_key(),
//!     bob.public_key(),
//!     "direct-chat",
//! ).unwrap();
//! let bob_shared = session::derive_shared_secret(
//!     bob.secret_key(),
//!     alice.public_key(),
//!     "direct-chat",
//! ).unwrap();
//! assert_eq!(alice_shared, bob_shared);
//!
//! // Alice шифрует
//! let payload = cipher::encrypt(b"Hello, Bob!", &alice_shared).unwrap();
//!
//! // Bob расшифровывает
//! let plaintext = cipher::decrypt(&payload, &bob_shared).unwrap();
//! assert_eq!(plaintext, b"Hello, Bob!");
//! ```

pub mod cipher;
pub mod disguise;
pub mod export;
pub mod group;
pub mod handshake;
pub mod keypair;
pub mod session;

/// Ошибки крейта
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Ошибка шифрования: {0}")]
    Encrypt(String),

    #[error("Ошибка расшифровки (неверный ключ или повреждённые данные)")]
    Decrypt,

    #[error("Неверный публичный ключ: {0}")]
    InvalidPublicKey(String),

    #[error("Неверный приватный ключ")]
    InvalidSecretKey,

    #[error("Неверная подпись")]
    InvalidSignature,

    #[error("Ошибка экспорта ключа: {0}")]
    Export(String),

    #[error("Ошибка импорта ключа: неверный пароль или повреждённые данные")]
    Import,

    #[error("Неверная мнемоника: {0}")]
    InvalidMnemonic(String),

    #[error("Участник группы не найден: {0}")]
    MemberNotFound(String),

    #[error("Ошибка сериализации: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Удобный реэкспорт основных типов
pub mod prelude {
    pub use crate::Error;
    pub use crate::Result;
    pub use crate::cipher::{self, EncryptedPayload};
    pub use crate::disguise;
    pub use crate::export::{ExportFormat, KeyExport};
    pub use crate::group::{GroupCipher, GroupEncryptedPayload};
    pub use crate::handshake::{HandshakeMessage, HandshakeState};
    pub use crate::keypair::IdentityKeypair;
    pub use crate::session;
}
