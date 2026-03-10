//! Вычисление общего секрета через X25519 Diffie-Hellman.
//!
//! Общий секрет одинаков для обеих сторон:
//! ```text
//! Alice: DH(alice_secret, bob_public)  == shared_secret
//! Bob:   DH(bob_secret,   alice_public) == shared_secret
//! ```
//!
//! Сырой DH-результат пропускается через HKDF-SHA256 для получения
//! равномерно распределённого ключа нужной длины.

use sha2::Sha256;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret as X25519SecretKey};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{Error, Result};

/// Общий секрет 32 байта — используется как ключ для ChaCha20-Poly1305.
#[derive(Clone, Zeroize, ZeroizeOnDrop, PartialEq)]
pub struct SharedSecret([u8; 32]);

impl SharedSecret {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for SharedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SharedSecret([REDACTED])")
    }
}

/// Вычисляет общий секрет из пары ключей.
///
/// `context` — произвольная строка для разделения доменов:
/// - `"direct-chat"` — для личных переписок
/// - `"group-{uuid}"` — для группового чата
///
/// Один и тот же DH-результат с разным context даёт разные ключи.
pub fn derive_shared_secret(
    my_secret: &X25519SecretKey,
    their_public: &X25519PublicKey,
    context: &str,
) -> Result<SharedSecret> {
    // X25519 DH
    let dh_result = my_secret.diffie_hellman(their_public);

    // Проверяем что результат не нулевой (атака с малым подгруппой)
    if dh_result.as_bytes().iter().all(|&b| b == 0) {
        return Err(Error::InvalidPublicKey(
            "DH результат нулевой — возможна атака".into(),
        ));
    }

    // HKDF-SHA256: извлекаем и расширяем ключевой материал
    use hkdf::Hkdf;
    let hk = Hkdf::<Sha256>::new(
        Some(b"email-chat-v1"), // salt
        dh_result.as_bytes(),   // ikm (input key material)
    );

    let mut okm = [0u8; 32];
    hk.expand(context.as_bytes(), &mut okm)
        .map_err(|_| Error::Encrypt("HKDF expand failed".into()))?;

    Ok(SharedSecret(okm))
}

/// Вычисляет shared secret из байтов публичного ключа.
/// Удобная обёртка для использования в других модулях.
pub fn derive_from_bytes(
    my_secret: &X25519SecretKey,
    their_public_bytes: &[u8; 32],
    context: &str,
) -> Result<SharedSecret> {
    let their_public = X25519PublicKey::from(*their_public_bytes);
    derive_shared_secret(my_secret, &their_public, context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keypair::IdentityKeypair;

    #[test]
    fn test_dh_is_symmetric() {
        let alice = IdentityKeypair::generate();
        let bob = IdentityKeypair::generate();

        let alice_shared =
            derive_shared_secret(alice.secret_key(), bob.public_key(), "direct-chat").unwrap();

        let bob_shared =
            derive_shared_secret(bob.secret_key(), alice.public_key(), "direct-chat").unwrap();

        assert_eq!(alice_shared.as_bytes(), bob_shared.as_bytes());
    }

    #[test]
    fn test_different_context_gives_different_secret() {
        let alice = IdentityKeypair::generate();
        let bob = IdentityKeypair::generate();

        let s1 = derive_shared_secret(alice.secret_key(), bob.public_key(), "ctx-1").unwrap();
        let s2 = derive_shared_secret(alice.secret_key(), bob.public_key(), "ctx-2").unwrap();

        assert_ne!(s1.as_bytes(), s2.as_bytes());
    }
}
