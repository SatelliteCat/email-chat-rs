//! iOS Keychain Services.
//!
//! ## Как это работает на iOS
//!
//! iOS Keychain — защищённое хранилище, привязанное к App ID.
//! Данные шифруются hardware-ключами Secure Enclave.
//! Доступны только приложению (или apps в одном App Group).
//!
//! ## Текущий статус: STUB
//!
//! Реальная реализация требует вызовов через `objc` crate или Swift bridge:
//!
//! ```text
//! Rust (platform/keystore/ios.rs)
//!     │  objc / Swift bridge
//!     ▼
//! Swift (ios/Sources/KeychainBridge.swift)
//!     │  SecItemAdd / SecItemCopyMatching / SecItemDelete
//!     ▼
//! iOS Keychain Services (Secure Enclave)
//! ```
//!
//! При разработке мобильной версии здесь будет вызов через uniffi или objc2:
//! ```rust
//! use objc2_security::{SecItem, kSecClass, kSecAttrAccount, ...};
//! ```

use async_trait::async_trait;
use echat_core::Result;
use echat_core::ports::keystore::KeystorePort;

/// iOS Keychain реализация (stub — требует objc/Swift bridge при сборке под iOS).
pub struct IosKeystore;

impl IosKeystore {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self)
    }
}

#[async_trait]
impl KeystorePort for IosKeystore {
    async fn set(&self, service: &str, key: &str, secret: &[u8]) -> Result<()> {
        // TODO: Swift bridge → KeychainBridge.set(service: key: data:)
        tracing::warn!("IosKeystore::set — stub, данные не сохранены");
        Err(core::Error::Keystore(
            "IosKeystore не реализован (stub). Требует Swift bridge.".into(),
        ))
    }

    async fn get(&self, service: &str, key: &str) -> Result<Option<Vec<u8>>> {
        // TODO: Swift bridge → KeychainBridge.get(service: key:)
        tracing::warn!("IosKeystore::get — stub");
        Ok(None)
    }

    async fn delete(&self, service: &str, key: &str) -> Result<()> {
        // TODO: Swift bridge → KeychainBridge.delete(service: key:)
        tracing::warn!("IosKeystore::delete — stub");
        Ok(())
    }
}
