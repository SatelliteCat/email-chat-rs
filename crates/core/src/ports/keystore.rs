//! Трейт KeystorePort — безопасное хранение секретов.
//!
//! Реализации:
//! - Desktop: OS keychain через `keyring` crate
//! - Android: Android Keystore System (JNI)
//! - iOS: Keychain Services API (через objc)
//! - Tests: in-memory HashMap

use std::sync::Arc;

use async_trait::async_trait;

use crate::Result;

/// Абстракция над платформенным хранилищем секретов.
///
/// Хранит пары (service, key) → secret_bytes.
/// Всё что не должно попасть в SQLite — идёт сюда:
/// - app_password для IMAP/SMTP
/// - приватный ключ identity keypair
#[async_trait]
pub trait KeystorePort: Send + Sync + 'static {
    /// Сохраняет секрет. Перезаписывает если уже есть.
    async fn set(&self, service: &str, key: &str, secret: &[u8]) -> Result<()>;

    /// Получает секрет. Возвращает None если не найден.
    async fn get(&self, service: &str, key: &str) -> Result<Option<Vec<u8>>>;

    /// Удаляет секрет.
    async fn delete(&self, service: &str, key: &str) -> Result<()>;
}

pub type DynKeystore = Arc<dyn KeystorePort>;

// ── Константы ключей ─────────────────────────────────────────────────────────

/// Сервис для app_password'ов почтовых аккаунтов.
pub const SERVICE_MAIL: &str = "echat.mail";

/// Сервис для identity keypair (приватный ключ шифрования).
pub const SERVICE_IDENTITY: &str = "echat.identity";

/// Ключ для приватного identity seed конкретного аккаунта.
/// Формат: `identity_seed:{account_id}`
pub fn identity_seed_key(account_id: &str) -> String {
    format!("identity_seed:{}", account_id)
}

/// Ключ для app_password конкретного аккаунта.
/// Формат: `app_password:{account_email}`
pub fn app_password_key(email: &str) -> String {
    format!("app_password:{}", email)
}

// ── In-memory реализация для тестов ──────────────────────────────────────────

use std::collections::HashMap;
use tokio::sync::RwLock;

/// In-memory keystore — только для тестов.
#[derive(Default)]
pub struct InMemoryKeystore {
    store: RwLock<HashMap<(String, String), Vec<u8>>>,
}

impl InMemoryKeystore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

#[async_trait]
impl KeystorePort for InMemoryKeystore {
    async fn set(&self, service: &str, key: &str, secret: &[u8]) -> Result<()> {
        self.store
            .write()
            .await
            .insert((service.to_string(), key.to_string()), secret.to_vec());
        Ok(())
    }

    async fn get(&self, service: &str, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .store
            .read()
            .await
            .get(&(service.to_string(), key.to_string()))
            .cloned())
    }

    async fn delete(&self, service: &str, key: &str) -> Result<()> {
        self.store
            .write()
            .await
            .remove(&(service.to_string(), key.to_string()));
        Ok(())
    }
}
