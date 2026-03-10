//! Генерация и управление ключами.
//!
//! Каждый пользователь имеет одну `IdentityKeypair`, которая содержит:
//! - **Ed25519** ключ — для цифровой подписи handshake-сообщений
//! - **X25519** ключ — для Diffie-Hellman обмена и шифрования
//!
//! Оба ключа получаются детерминированно из одного 32-байтного seed,
//! что позволяет хранить и восстанавливать одним бэкапом.

use rand::RngCore;
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret as X25519SecretKey};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{Error, Result};

/// Seed — 32 байта из которых получаются все ключи.
/// Zeroize очищает память при drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct KeySeed([u8; 32]);

impl KeySeed {
    /// Генерирует новый случайный seed через OS CSPRNG.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

/// Публичная часть ключевой пары — безопасно сериализуется и передаётся.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicKeys {
    /// X25519 публичный ключ — для DH обмена (32 байта)
    pub x25519: [u8; 32],
    /// Ed25519 публичный ключ — для проверки подписей (32 байта)
    pub ed25519: [u8; 32],
}

impl PublicKeys {
    /// Кодирует публичные ключи в base64 для передачи в handshake.
    pub fn to_base64(&self) -> Result<String> {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let bytes = serde_json::to_vec(self)?;
        Ok(STANDARD.encode(bytes))
    }

    /// Декодирует из base64.
    pub fn from_base64(s: &str) -> Result<Self> {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let bytes = STANDARD
            .decode(s)
            .map_err(|e| Error::InvalidPublicKey(e.to_string()))?;
        serde_json::from_slice(&bytes).map_err(Error::Serialization)
    }
}

/// Полная ключевая пара пользователя.
///
/// Приватная часть никогда не покидает эту структуру напрямую —
/// только через [`export`](crate::export) с защитой паролем.
pub struct IdentityKeypair {
    seed: KeySeed,
    x25519_secret: X25519SecretKey,
    x25519_public: X25519PublicKey,
    ed25519_signing: ed25519_dalek::SigningKey,
}

impl IdentityKeypair {
    /// Генерирует новую ключевую пару.
    pub fn generate() -> Self {
        let seed = KeySeed::generate();
        Self::from_seed(seed)
    }

    /// Восстанавливает ключевую пару из seed.
    pub fn from_seed(seed: KeySeed) -> Self {
        // X25519: используем seed напрямую
        let x25519_secret = X25519SecretKey::from(*seed.as_bytes());
        let x25519_public = X25519PublicKey::from(&x25519_secret);

        // Ed25519: хэшируем seed чтобы получить независимый ключ
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(b"ed25519-derive-v1");
        hasher.update(seed.as_bytes());
        let ed_seed: [u8; 32] = hasher.finalize().into();
        let ed25519_signing = ed25519_dalek::SigningKey::from_bytes(&ed_seed);

        Self {
            seed,
            x25519_secret,
            x25519_public,
            ed25519_signing,
        }
    }

    /// Возвращает публичные ключи для передачи собеседнику.
    pub fn public_keys(&self) -> PublicKeys {
        PublicKeys {
            x25519: *self.x25519_public.as_bytes(),
            ed25519: self.ed25519_signing.verifying_key().to_bytes(),
        }
    }

    /// Возвращает X25519 публичный ключ (для DH).
    pub fn public_key(&self) -> &X25519PublicKey {
        &self.x25519_public
    }

    /// Возвращает X25519 приватный ключ (для DH).
    /// Только для использования внутри крейта encryption.
    pub(crate) fn secret_key(&self) -> &X25519SecretKey {
        &self.x25519_secret
    }

    /// Возвращает Ed25519 signing key (для подписи handshake).
    pub(crate) fn signing_key(&self) -> &ed25519_dalek::SigningKey {
        &self.ed25519_signing
    }

    /// Возвращает seed для экспорта (только через защищённый export модуль).
    pub(crate) fn seed(&self) -> &KeySeed {
        &self.seed
    }

    /// Подписывает данные Ed25519.
    pub fn sign(&self, data: &[u8]) -> [u8; 64] {
        use ed25519_dalek::Signer;
        self.ed25519_signing.sign(data).to_bytes()
    }

    /// Проверяет подпись по публичному Ed25519 ключу.
    pub fn verify(pubkey_bytes: &[u8; 32], data: &[u8], signature: &[u8; 64]) -> Result<()> {
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};

        let verifying_key = VerifyingKey::from_bytes(pubkey_bytes)
            .map_err(|_| Error::InvalidPublicKey("неверный Ed25519 ключ".into()))?;

        let sig = Signature::from_bytes(signature);

        verifying_key
            .verify(data, &sig)
            .map_err(|_| Error::InvalidSignature)
    }
}

impl Drop for IdentityKeypair {
    fn drop(&mut self) {
        // seed и x25519_secret зануляются автоматически через Zeroize
        // ed25519_signing — zeroize вручную
        use zeroize::Zeroize;
        let mut bytes = self.ed25519_signing.to_bytes();
        bytes.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_is_unique() {
        let kp1 = IdentityKeypair::generate();
        let kp2 = IdentityKeypair::generate();
        assert_ne!(kp1.public_keys().x25519, kp2.public_keys().x25519);
    }

    #[test]
    fn test_from_seed_is_deterministic() {
        let seed = KeySeed::generate();
        let seed_bytes = *seed.as_bytes();

        let kp1 = IdentityKeypair::from_seed(KeySeed::from_bytes(seed_bytes));
        let kp2 = IdentityKeypair::from_seed(KeySeed::from_bytes(seed_bytes));

        assert_eq!(kp1.public_keys(), kp2.public_keys());
    }

    #[test]
    fn test_sign_and_verify() {
        let kp = IdentityKeypair::generate();
        let data = b"test message";

        let sig = kp.sign(data);
        let pubkey = kp.public_keys();

        assert!(IdentityKeypair::verify(&pubkey.ed25519, data, &sig).is_ok());
        assert!(IdentityKeypair::verify(&pubkey.ed25519, b"wrong data", &sig).is_err());
    }

    #[test]
    fn test_public_keys_base64_roundtrip() {
        let kp = IdentityKeypair::generate();
        let pubkeys = kp.public_keys();

        let encoded = pubkeys.to_base64().unwrap();
        let decoded = PublicKeys::from_base64(&encoded).unwrap();

        assert_eq!(pubkeys, decoded);
    }
}
