//! Шифрование и расшифрование сообщений.
//!
//! Используется **ChaCha20-Poly1305** (AEAD):
//! - ChaCha20 — потоковый шифр (конфиденциальность)
//! - Poly1305 — MAC (целостность + аутентификация)
//!
//! Каждое сообщение получает уникальный случайный nonce (96 бит).
//! Повторное использование nonce с одним ключом катастрофично —
//! поэтому nonce генерируется через OS CSPRNG.

use chacha20poly1305::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    ChaCha20Poly1305, Nonce,
};
use serde::{Deserialize, Serialize};

use crate::{session::SharedSecret, Error, Result};

/// Magic bytes — первые 4 байта любого нашего сообщения.
/// Позволяет быстро определить "наше ли это письмо" без расшифровки.
pub const MAGIC: [u8; 4] = [0xEC, 0xC4, 0xA7, 0x01];

/// Версия формата payload.
pub const PAYLOAD_VERSION: u8 = 1;

/// Зашифрованный payload — то что кладётся в тело письма.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedPayload {
    /// Версия протокола (для будущей совместимости)
    pub version: u8,
    /// Случайный nonce 12 байт
    pub nonce: [u8; 12],
    /// Зашифрованные данные (ciphertext + 16 байт Poly1305 tag)
    pub ciphertext: Vec<u8>,
}

impl EncryptedPayload {
    /// Кодирует в base64 для вставки в тело письма.
    pub fn to_base64(&self) -> String {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let mut bytes = Vec::with_capacity(4 + 1 + 12 + self.ciphertext.len());
        bytes.extend_from_slice(&MAGIC);
        bytes.push(self.version);
        bytes.extend_from_slice(&self.nonce);
        bytes.extend_from_slice(&self.ciphertext);
        STANDARD.encode(bytes)
    }

    /// Декодирует из base64.
    pub fn from_base64(s: &str) -> Result<Self> {
        use base64::{engine::general_purpose::STANDARD, Engine};

        let sanitized: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        let bytes = STANDARD
            .decode(&sanitized)
            .map_err(|_| Error::Decrypt)?;

        if bytes.len() < 4 + 1 + 12 + 16 {
            return Err(Error::Decrypt);
        }

        // Проверяем magic bytes
        if bytes[..4] != MAGIC {
            return Err(Error::Decrypt);
        }

        let version = bytes[4];
        let nonce: [u8; 12] = bytes[5..17].try_into().map_err(|_| Error::Decrypt)?;
        let ciphertext = bytes[17..].to_vec();

        Ok(Self {
            version,
            nonce,
            ciphertext,
        })
    }

    /// Быстрая проверка: содержит ли строка наш magic (до декодирования).
    /// Используется для фильтрации писем в SyncEngine.
    pub fn has_magic_prefix(base64_body: &str) -> bool {
        use base64::{engine::general_purpose::STANDARD, Engine};
        // Первые ~8 символов base64 соответствуют первым 6 байтам
        let prefix = &base64_body.trim()[..base64_body.len().min(8)];
        if let Ok(bytes) = STANDARD.decode(prefix) {
            bytes.starts_with(&MAGIC)
        } else {
            false
        }
    }
}

/// Шифрует plaintext с помощью shared secret.
pub fn encrypt(plaintext: &[u8], secret: &SharedSecret) -> Result<EncryptedPayload> {
    let key = chacha20poly1305::Key::from_slice(secret.as_bytes());
    let cipher = ChaCha20Poly1305::new(key);

    let nonce_bytes = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let nonce: [u8; 12] = nonce_bytes.into();

    let ciphertext = cipher
        .encrypt(&Nonce::from(nonce), plaintext)
        .map_err(|e| Error::Encrypt(e.to_string()))?;

    Ok(EncryptedPayload {
        version: PAYLOAD_VERSION,
        nonce,
        ciphertext,
    })
}

/// Расшифровывает payload с помощью shared secret.
pub fn decrypt(payload: &EncryptedPayload, secret: &SharedSecret) -> Result<Vec<u8>> {
    if payload.version != PAYLOAD_VERSION {
        return Err(Error::Decrypt);
    }

    let key = chacha20poly1305::Key::from_slice(secret.as_bytes());
    let cipher = ChaCha20Poly1305::new(key);
    let nonce = Nonce::from(payload.nonce);

    cipher
        .decrypt(&nonce, payload.ciphertext.as_slice())
        .map_err(|_| Error::Decrypt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{keypair::IdentityKeypair, session::derive_shared_secret};

    fn make_shared_secret() -> SharedSecret {
        let alice = IdentityKeypair::generate();
        let bob = IdentityKeypair::generate();
        derive_shared_secret(alice.secret_key(), bob.public_key(), "direct-chat").unwrap()
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let alice = IdentityKeypair::generate();
        let bob = IdentityKeypair::generate();

        let alice_secret =
            derive_shared_secret(alice.secret_key(), bob.public_key(), "direct-chat").unwrap();
        let bob_secret =
            derive_shared_secret(bob.secret_key(), alice.public_key(), "direct-chat").unwrap();

        let plaintext = b"Hello, Bob! This is a secret message.";
        let payload = encrypt(plaintext, &alice_secret).unwrap();
        let decrypted = decrypt(&payload, &bob_secret).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_wrong_key_fails() {
        let secret = make_shared_secret();
        let wrong_secret = make_shared_secret();

        let payload = encrypt(b"secret", &secret).unwrap();
        assert!(decrypt(&payload, &wrong_secret).is_err());
    }

    #[test]
    fn test_tampered_ciphertext_fails() {
        let secret = make_shared_secret();
        let mut payload = encrypt(b"secret", &secret).unwrap();

        // Портим один байт
        payload.ciphertext[0] ^= 0xFF;
        assert!(decrypt(&payload, &secret).is_err());
    }

    #[test]
    fn test_base64_roundtrip() {
        let secret = make_shared_secret();
        let payload = encrypt(b"test message", &secret).unwrap();

        let encoded = payload.to_base64();
        let decoded = EncryptedPayload::from_base64(&encoded).unwrap();

        assert_eq!(payload.nonce, decoded.nonce);
        assert_eq!(payload.ciphertext, decoded.ciphertext);
    }

    #[test]
    fn test_magic_prefix_detection() {
        let secret = make_shared_secret();
        let payload = encrypt(b"msg", &secret).unwrap();
        let b64 = payload.to_base64();

        assert!(EncryptedPayload::has_magic_prefix(&b64));
        assert!(!EncryptedPayload::has_magic_prefix("aGVsbG8gd29ybGQ="));
    }

    #[test]
    fn test_nonce_is_unique() {
        let secret = make_shared_secret();
        let p1 = encrypt(b"msg", &secret).unwrap();
        let p2 = encrypt(b"msg", &secret).unwrap();

        // Одинаковый plaintext → разные nonce
        assert_ne!(p1.nonce, p2.nonce);
        assert_ne!(p1.ciphertext, p2.ciphertext);
    }
}
