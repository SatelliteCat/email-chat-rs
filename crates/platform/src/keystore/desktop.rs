//! Desktop keystore — Windows Credential Manager / macOS Keychain / Secret Service.
//!
//! Использует крейт `keyring` который скрывает разницу между платформами.
//!
//! ## Формат ключей в keyring
//!
//! keyring хранит записи как (service, username) → password.
//! Мы кодируем бинарные данные в base64 чтобы хранить их как "пароль".
//!
//! Итоговый ключ: service = "echat.{service}", username = "{key}"

use async_trait::async_trait;
use echat_core::Result;
use echat_core::ports::keystore::KeystorePort;

/// Реализация KeystorePort для desktop через OS keychain.
pub struct DesktopKeystore;

impl DesktopKeystore {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self)
    }
}

#[async_trait]
impl KeystorePort for DesktopKeystore {
    async fn set(&self, service: &str, key: &str, secret: &[u8]) -> Result<()> {
        use base64::{Engine, engine::general_purpose::STANDARD};

        let full_service = format!("echat.{}", service);
        let encoded = STANDARD.encode(secret);

        tracing::info!("Keystore SET: service={}, key={}", full_service, key);

        // keyring — синхронный API, выносим в spawn_blocking
        let svc = full_service.clone();
        let k = key.to_string();
        tokio::task::spawn_blocking(move || {
            let entry = keyring::Entry::new(&svc, &k)
                .map_err(|e| echat_core::Error::Keystore(e.to_string()))?;
            entry
                .set_password(&encoded)
                .map_err(|e| echat_core::Error::Keystore(e.to_string()))
        })
        .await
        .map_err(|e| echat_core::Error::Keystore(e.to_string()))??;

        tracing::info!("Keystore SET OK: service={}, key={}", full_service, key);
        Ok(())
    }

    async fn get(&self, service: &str, key: &str) -> Result<Option<Vec<u8>>> {
        use base64::{Engine, engine::general_purpose::STANDARD};

        let full_service = format!("echat.{}", service);
        let k = key.to_string();

        tracing::info!("Keystore GET: service={}, key={}", full_service, key);

        let result = tokio::task::spawn_blocking(move || {
            let entry = keyring::Entry::new(&full_service, &k)
                .map_err(|e| echat_core::Error::Keystore(e.to_string()))?;
            match entry.get_password() {
                Ok(encoded) => Ok(Some(encoded)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(e) => Err(echat_core::Error::Keystore(e.to_string())),
            }
        })
        .await
        .map_err(|e| echat_core::Error::Keystore(e.to_string()))??;

        match result {
            None => {
                Ok(None)
            }
            Some(encoded) => {
                tracing::info!("Keystore GET OK: service={}, key={}", service, key);
                let bytes = STANDARD
                    .decode(&encoded)
                    .map_err(|e| echat_core::Error::Keystore(format!("base64 decode: {}", e)))?;
                Ok(Some(bytes))
            }
        }
    }

    async fn delete(&self, service: &str, key: &str) -> Result<()> {
        let full_service = format!("echat.{}", service);
        let k = key.to_string();

        tokio::task::spawn_blocking(move || {
            let entry = keyring::Entry::new(&full_service, &k)
                .map_err(|e| echat_core::Error::Keystore(e.to_string()))?;
            match entry.delete_credential() {
                Ok(()) => Ok(()),
                // Если записи нет — не ошибка
                Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(echat_core::Error::Keystore(e.to_string())),
            }
        })
        .await
        .map_err(|e| echat_core::Error::Keystore(e.to_string()))?
    }
}
