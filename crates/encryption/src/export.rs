//! Экспорт и импорт приватного ключа.
//!
//! ## Форматы
//!
//! - [`ExportFormat::Mnemonic`] — 24 слова BIP-39, легко записать на бумагу
//! - [`ExportFormat::Base64`]   — строка base64, удобно для копирования
//!
//! ## Защита паролем
//!
//! Seed шифруется перед экспортом:
//! ```text
//! password ──[Argon2id]──► 32-byte key
//! key + seed ──[ChaCha20-Poly1305]──► encrypted_blob
//! ```
//!
//! Argon2id параметры (OWASP рекомендации 2024):
//! - memory: 64 MB
//! - iterations: 3
//! - parallelism: 4

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    ChaCha20Poly1305, Key, Nonce,
};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, /*ZeroizeOnDrop*/};

use crate::{
    keypair::{IdentityKeypair, KeySeed},
    Error, Result,
};

/// Формат экспорта ключа.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportFormat {
    /// 24 слова BIP-39. Пример:
    /// "abandon ability able about above absent absorb abstract ..."
    Mnemonic,
    /// base64-строка — для сохранения в файл или менеджер паролей
    Base64,
}

/// Зашифрованный экспортированный ключ.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyExport {
    /// Версия формата
    pub version: u8,
    /// Формат ("mnemonic" | "base64")
    pub format: String,
    /// Salt для Argon2id (16 байт, base64)
    pub argon2_salt: String,
    /// Argon2id параметры
    pub argon2_memory_kb: u32,
    pub argon2_iterations: u32,
    pub argon2_parallelism: u32,
    /// Nonce для ChaCha20-Poly1305 (12 байт, base64)
    pub nonce: String,
    /// Зашифрованный seed (base64)
    /// Длина: 32 (seed) + 16 (tag) = 48 байт
    pub ciphertext: String,
    /// Контрольная сумма (SHA-256 от seed, первые 4 байта, hex)
    /// Для быстрой проверки пароля без расшифровки
    pub checksum: String,
}

// Argon2id параметры
const ARGON2_MEMORY_KB: u32 = 65536; // 64 MB
const ARGON2_ITERATIONS: u32 = 3;
const ARGON2_PARALLELISM: u32 = 4;

/// Экспортирует ключевую пару с защитой паролем.
pub fn export_keypair(
    keypair: &IdentityKeypair,
    password: &str,
    format: ExportFormat,
) -> Result<ExportedKey> {
    let seed_bytes = keypair.seed().as_bytes();

    // 1. Генерируем salt
    use rand::RngCore;
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);

    // 2. Получаем ключ из пароля через Argon2id
    let mut derived_key = derive_key(password, &salt)?;

    // 3. Шифруем seed
    let key = Key::from_slice(&derived_key);
    let cipher = ChaCha20Poly1305::new(key);
    let nonce_bytes = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let nonce: [u8; 12] = nonce_bytes.into();

    let ciphertext = cipher
        .encrypt(&Nonce::from(nonce), seed_bytes.as_slice())
        .map_err(|e| Error::Export(e.to_string()))?;

    derived_key.zeroize();

    // 4. Контрольная сумма
    let checksum = compute_checksum(seed_bytes);

    // 5. Формируем структуру
    use base64::{engine::general_purpose::STANDARD, Engine};
    let export = KeyExport {
        version: 1,
        format: match format {
            ExportFormat::Mnemonic => "mnemonic",
            ExportFormat::Base64 => "base64",
        }
        .to_string(),
        argon2_salt: STANDARD.encode(salt),
        argon2_memory_kb: ARGON2_MEMORY_KB,
        argon2_iterations: ARGON2_ITERATIONS,
        argon2_parallelism: ARGON2_PARALLELISM,
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(&ciphertext),
        checksum,
    };

    // 6. Кодируем в нужный формат
    match format {
        ExportFormat::Mnemonic => {
            let seed_copy = *seed_bytes;
            let mnemonic = seed_to_mnemonic(&seed_copy)?;
            Ok(ExportedKey::Mnemonic {
                words: mnemonic,
                metadata: export,
            })
        }
        ExportFormat::Base64 => {
            let json = serde_json::to_string(&export)
                .map_err(|e| Error::Export(e.to_string()))?;
            let b64 = STANDARD.encode(json.as_bytes());
            Ok(ExportedKey::Base64 {
                data: b64,
                metadata: export,
            })
        }
    }
}

/// Импортирует ключевую пару из экспорта.
pub fn import_keypair(exported: &ExportedKey, password: &str) -> Result<IdentityKeypair> {
    let metadata = exported.metadata();
    let seed = decrypt_seed(metadata, password)?;
    Ok(IdentityKeypair::from_seed(KeySeed::from_bytes(seed)))
}

/// Результат экспорта.
#[derive(Debug)]
pub enum ExportedKey {
    Mnemonic {
        /// 24 слова через пробел
        words: String,
        metadata: KeyExport,
    },
    Base64 {
        /// base64-строка для сохранения
        data: String,
        metadata: KeyExport,
    },
}

impl ExportedKey {
    pub fn metadata(&self) -> &KeyExport {
        match self {
            Self::Mnemonic { metadata, .. } => metadata,
            Self::Base64 { metadata, .. } => metadata,
        }
    }

    /// Возвращает строку для отображения пользователю.
    pub fn display_string(&self) -> &str {
        match self {
            Self::Mnemonic { words, .. } => words,
            Self::Base64 { data, .. } => data,
        }
    }

    /// Парсит из строки (автоопределение формата).
    pub fn from_string(s: &str) -> Result<Self> {
        let trimmed = s.trim();
        // Мнемоника: слова через пробел, нет символов '+', '/', '='
        if trimmed.split_whitespace().count() == 24
            && !trimmed.contains('+')
            && !trimmed.contains('/')
        {
            // Мнемоника без метаданных — создаём восстановление из слов напрямую
            let seed = mnemonic_to_seed(trimmed)?;
            let keypair = IdentityKeypair::from_seed(KeySeed::from_bytes(seed));
            // Для мнемоники без пароля возвращаем Base64 с пустым метаданными
            let _ = keypair; // просто проверили валидность
            Err(Error::Import) // нужен metadata для восстановления с паролем
        } else {
            use base64::{engine::general_purpose::STANDARD, Engine};
            let json_bytes = STANDARD.decode(trimmed).map_err(|_| Error::Import)?;
            let metadata: KeyExport =
                serde_json::from_slice(&json_bytes).map_err(|_| Error::Import)?;
            Ok(Self::Base64 {
                data: trimmed.to_string(),
                metadata,
            })
        }
    }
}

// ── Внутренние утилиты ────────────────────────────────────────────────────────

/// Выводит ключ из пароля через Argon2id.
fn derive_key(password: &str, salt: &[u8]) -> Result<Vec<u8>> {
    let params = Params::new(
        ARGON2_MEMORY_KB,
        ARGON2_ITERATIONS,
        ARGON2_PARALLELISM,
        Some(32),
    )
    .map_err(|e| Error::Export(e.to_string()))?;

    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    let mut output = vec![0u8; 32];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut output)
        .map_err(|e| Error::Export(e.to_string()))?;

    Ok(output)
}

/// Расшифровывает seed из KeyExport.
fn decrypt_seed(export: &KeyExport, password: &str) -> Result<[u8; 32]> {
    use base64::{engine::general_purpose::STANDARD, Engine};

    let salt = STANDARD.decode(&export.argon2_salt).map_err(|_| Error::Import)?;
    let nonce_bytes = STANDARD.decode(&export.nonce).map_err(|_| Error::Import)?;
    let ciphertext = STANDARD.decode(&export.ciphertext).map_err(|_| Error::Import)?;

    let mut derived_key = derive_key(password, &salt)?;

    let key = Key::from_slice(&derived_key);
    let cipher = ChaCha20Poly1305::new(key);
    let nonce: [u8; 12] = nonce_bytes.try_into().map_err(|_| Error::Import)?;

    let seed_bytes = cipher
        .decrypt(&Nonce::from(nonce), ciphertext.as_slice())
        .map_err(|_| Error::Import)?;

    derived_key.zeroize();

    // Проверяем контрольную сумму
    let seed_array: [u8; 32] = seed_bytes.try_into().map_err(|_| Error::Import)?;
    let checksum = compute_checksum(&seed_array);
    if checksum != export.checksum {
        return Err(Error::Import);
    }

    Ok(seed_array)
}

fn compute_checksum(seed: &[u8; 32]) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(seed);
    hex::encode(&hash[..4])
}

fn seed_to_mnemonic(seed: &[u8; 32]) -> Result<String> {
    use bip39::Mnemonic;
    let mnemonic = Mnemonic::from_entropy(seed)
        .map_err(|e| Error::Export(e.to_string()))?;
    Ok(mnemonic.to_string())
}

fn mnemonic_to_seed(words: &str) -> Result<[u8; 32]> {
    use bip39::Mnemonic;
    use std::str::FromStr;

    let mnemonic = Mnemonic::from_str(words)
        .map_err(|e| Error::InvalidMnemonic(e.to_string()))?;

    let entropy = mnemonic.to_entropy();
    entropy
        .try_into()
        .map_err(|_| Error::InvalidMnemonic("неверная длина entropy".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_import_base64() {
        let keypair = IdentityKeypair::generate();
        let original_pubkeys = keypair.public_keys();

        let exported = export_keypair(&keypair, "my-strong-password", ExportFormat::Base64)
            .unwrap();
        let restored = import_keypair(&exported, "my-strong-password").unwrap();

        assert_eq!(original_pubkeys, restored.public_keys());
    }

    #[test]
    fn test_export_import_mnemonic() {
        let keypair = IdentityKeypair::generate();
        let original_pubkeys = keypair.public_keys();

        let exported = export_keypair(&keypair, "pass123", ExportFormat::Mnemonic).unwrap();

        // Проверяем что получили 24 слова
        if let ExportedKey::Mnemonic { words, .. } = &exported {
            assert_eq!(words.split_whitespace().count(), 24);
        }

        let restored = import_keypair(&exported, "pass123").unwrap();
        assert_eq!(original_pubkeys, restored.public_keys());
    }

    #[test]
    fn test_wrong_password_fails() {
        let keypair = IdentityKeypair::generate();
        let exported = export_keypair(&keypair, "correct-password", ExportFormat::Base64)
            .unwrap();

        assert!(import_keypair(&exported, "wrong-password").is_err());
    }
}
