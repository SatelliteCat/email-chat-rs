//! Fan-out шифрование для групповых чатов.
//!
//! ## Схема
//!
//! Каждое групповое сообщение:
//! 1. Шифруется одним случайным `session_key` (ChaCha20-Poly1305)
//! 2. `session_key` шифруется отдельно для каждого участника (X25519 + HKDF)
//!
//! ```text
//! plaintext ──[session_key]──► ciphertext
//!
//! session_key ──[DH(alice, bob)]──► key_for_bob   (48 bytes)
//! session_key ──[DH(alice, carol)]──► key_for_carol
//! session_key ──[DH(alice, dave)]──► key_for_dave
//! ```
//!
//! При получении: Bob находит свой email в `member_keys`,
//! расшифровывает `session_key`, расшифровывает тело.

use std::collections::HashMap;

use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_with::{Bytes, serde_as};
use x25519_dalek::PublicKey as X25519PublicKey;

use crate::{Error, Result, keypair::IdentityKeypair, session};

/// Зашифрованное групповое сообщение.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupEncryptedPayload {
    /// Версия протокола
    pub version: u8,
    /// Nonce для основного шифрования
    pub nonce: [u8; 12],
    /// Зашифрованное тело сообщения (session_key)
    pub ciphertext: Vec<u8>,
    /// Зашифрованный session_key для каждого участника
    /// Ключ: email участника, значение: зашифрованный session_key
    pub member_keys: HashMap<String, EncryptedSessionKey>,
}

impl GroupEncryptedPayload {
    pub fn to_base64(&self) -> Result<String> {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let bytes = serde_json::to_vec(self)?;
        Ok(STANDARD.encode(bytes))
    }

    pub fn from_base64(s: &str) -> Result<Self> {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let bytes = STANDARD.decode(s.trim()).map_err(|_| Error::Decrypt)?;
        serde_json::from_slice(&bytes).map_err(Error::Serialization)
    }
}

/// Session key зашифрованный для одного участника.
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedSessionKey {
    pub nonce: [u8; 12],
    /// 32 байта session_key + 16 байт Poly1305 tag = 48 байт
    #[serde_as(as = "Bytes")]
    pub ciphertext: [u8; 48],
}

/// Участник группы — email + X25519 публичный ключ.
pub struct GroupMember {
    pub email: String,
    pub public_key_bytes: [u8; 32],
}

/// Шифрует сообщение для группы участников.
///
/// `sender` — отправитель (нужен для DH с каждым участником).
/// `members` — список всех получателей (включая себя, чтобы прочитать своё сообщение).
pub fn encrypt(
    plaintext: &[u8],
    sender: &IdentityKeypair,
    group_id: &str,
    members: &[GroupMember],
) -> Result<GroupEncryptedPayload> {
    if members.is_empty() {
        return Err(Error::MemberNotFound("список участников пуст".into()));
    }

    // 1. Генерируем случайный session_key
    let mut session_key = [0u8; 32];
    OsRng.fill_bytes(&mut session_key);

    // 2. Шифруем тело сообщения session_key'ом
    let body_key = Key::from_slice(&session_key);
    let body_cipher = ChaCha20Poly1305::new(body_key);
    let body_nonce_bytes = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let body_nonce: [u8; 12] = body_nonce_bytes.into();

    let ciphertext = body_cipher
        .encrypt(&Nonce::from(body_nonce), plaintext)
        .map_err(|e| Error::Encrypt(e.to_string()))?;

    // 3. Шифруем session_key для каждого участника
    let context = format!("group-{}", group_id);
    let mut member_keys = HashMap::new();

    for member in members {
        let their_public = X25519PublicKey::from(member.public_key_bytes);
        let shared = session::derive_shared_secret(sender.secret_key(), &their_public, &context)?;

        let wrap_key = Key::from_slice(shared.as_bytes());
        let wrap_cipher = ChaCha20Poly1305::new(wrap_key);
        let wrap_nonce_bytes = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let wrap_nonce: [u8; 12] = wrap_nonce_bytes.into();

        let wrapped = wrap_cipher
            .encrypt(&Nonce::from(wrap_nonce), session_key.as_slice())
            .map_err(|e| Error::Encrypt(e.to_string()))?;

        // session_key (32) + tag (16) = 48 байт
        let ciphertext_array: [u8; 48] = wrapped
            .try_into()
            .map_err(|_| Error::Encrypt("неожиданная длина wrapped key".into()))?;

        member_keys.insert(
            member.email.clone(),
            EncryptedSessionKey {
                nonce: wrap_nonce,
                ciphertext: ciphertext_array,
            },
        );
    }

    // Зануляем session_key в памяти
    use zeroize::Zeroize;
    session_key.zeroize();

    Ok(GroupEncryptedPayload {
        version: 1,
        nonce: body_nonce,
        ciphertext,
        member_keys,
    })
}

/// Расшифровывает групповое сообщение.
///
/// `recipient` — ключевая пара получателя.
/// `recipient_email` — email получателя (для поиска в member_keys).
/// `sender_public_key` — X25519 публичный ключ отправителя.
pub fn decrypt(
    payload: &GroupEncryptedPayload,
    recipient: &IdentityKeypair,
    recipient_email: &str,
    sender_public_key_bytes: &[u8; 32],
    group_id: &str,
) -> Result<Vec<u8>> {
    // 1. Находим запись для нашего email
    let encrypted_key = payload
        .member_keys
        .get(recipient_email)
        .ok_or_else(|| Error::MemberNotFound(recipient_email.to_string()))?;

    // 2. Восстанавливаем shared secret с отправителем
    let context = format!("group-{}", group_id);
    let sender_public = X25519PublicKey::from(*sender_public_key_bytes);
    let shared = session::derive_shared_secret(recipient.secret_key(), &sender_public, &context)?;

    // 3. Расшифровываем session_key
    let wrap_key = Key::from_slice(shared.as_bytes());
    let wrap_cipher = ChaCha20Poly1305::new(wrap_key);
    let wrap_nonce = Nonce::from(encrypted_key.nonce);

    let mut session_key_bytes = wrap_cipher
        .decrypt(&wrap_nonce, encrypted_key.ciphertext.as_slice())
        .map_err(|_| Error::Decrypt)?;

    // 4. Расшифровываем тело
    let body_key = Key::from_slice(&session_key_bytes);
    let body_cipher = ChaCha20Poly1305::new(body_key);
    let body_nonce = Nonce::from(payload.nonce);

    let plaintext = body_cipher
        .decrypt(&body_nonce, payload.ciphertext.as_slice())
        .map_err(|_| Error::Decrypt)?;

    // Зануляем session_key
    use zeroize::Zeroize;
    session_key_bytes.zeroize();

    Ok(plaintext)
}

/// Удобная структура для работы с группой.
pub struct GroupCipher {
    pub group_id: String,
}

impl GroupCipher {
    pub fn new(group_id: &str) -> Self {
        Self {
            group_id: group_id.to_string(),
        }
    }

    pub fn encrypt(
        &self,
        plaintext: &[u8],
        sender: &IdentityKeypair,
        members: &[GroupMember],
    ) -> Result<GroupEncryptedPayload> {
        encrypt(plaintext, sender, &self.group_id, members)
    }

    pub fn decrypt(
        &self,
        payload: &GroupEncryptedPayload,
        recipient: &IdentityKeypair,
        recipient_email: &str,
        sender_public_key_bytes: &[u8; 32],
    ) -> Result<Vec<u8>> {
        decrypt(
            payload,
            recipient,
            recipient_email,
            sender_public_key_bytes,
            &self.group_id,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_member(keypair: &IdentityKeypair, email: &str) -> GroupMember {
        GroupMember {
            email: email.to_string(),
            public_key_bytes: *keypair.public_key().as_bytes(),
        }
    }

    #[test]
    fn test_group_encrypt_decrypt() {
        let alice = IdentityKeypair::generate();
        let bob = IdentityKeypair::generate();
        let carol = IdentityKeypair::generate();

        let members = vec![
            make_member(&alice, "alice@mail.ru"),
            make_member(&bob, "bob@yandex.ru"),
            make_member(&carol, "carol@mail.ru"),
        ];

        let group_id = "test-group-uuid";
        let plaintext = b"Hello, group!";

        // Alice отправляет
        let payload = encrypt(plaintext, &alice, group_id, &members).unwrap();

        // Bob расшифровывает
        let decrypted_by_bob = decrypt(
            &payload,
            &bob,
            "bob@yandex.ru",
            alice.public_key().as_bytes(),
            group_id,
        )
        .unwrap();
        assert_eq!(decrypted_by_bob, plaintext);

        // Carol расшифровывает
        let decrypted_by_carol = decrypt(
            &payload,
            &carol,
            "carol@mail.ru",
            alice.public_key().as_bytes(),
            group_id,
        )
        .unwrap();
        assert_eq!(decrypted_by_carol, plaintext);
    }

    #[test]
    fn test_non_member_cannot_decrypt() {
        let alice = IdentityKeypair::generate();
        let bob = IdentityKeypair::generate();
        let eve = IdentityKeypair::generate(); // не в группе

        let members = vec![make_member(&bob, "bob@yandex.ru")];

        let payload = encrypt(b"secret", &alice, "group-1", &members).unwrap();

        let result = decrypt(
            &payload,
            &eve,
            "eve@evil.com",
            alice.public_key().as_bytes(),
            "group-1",
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_base64_roundtrip() {
        let alice = IdentityKeypair::generate();
        let bob = IdentityKeypair::generate();
        let members = vec![make_member(&bob, "bob@yandex.ru")];

        let payload = encrypt(b"test", &alice, "grp", &members).unwrap();
        let b64 = payload.to_base64().unwrap();
        let decoded = GroupEncryptedPayload::from_base64(&b64).unwrap();

        assert_eq!(payload.nonce, decoded.nonce);
        assert_eq!(payload.ciphertext, decoded.ciphertext);
    }
}
