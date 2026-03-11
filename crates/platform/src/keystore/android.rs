//! Android Keystore System.
//!
//! ## Как это работает на Android
//!
//! Android Keystore хранит криптографические ключи в защищённом hardware-backed
//! хранилище. Для хранения произвольных байт (пароли, seed) используем
//! EncryptedSharedPreferences из Jetpack Security.
//!
//! ## Текущий статус: STUB
//!
//! Реальная реализация требует JNI-вызовов в Kotlin/Java код.
//! Архитектура:
//!
//! ```text
//! Rust (platform/keystore/android.rs)
//!     │  JNI call
//!     ▼
//! Kotlin (android/app/src/.../KeystoreBridge.kt)
//!     │  EncryptedSharedPreferences API
//!     ▼
//! Android Keystore System (hardware-backed)
//! ```
//!
//! При разработке мобильной версии здесь будет:
//! ```rust
//! extern "C" {
//!     fn Java_com_echat_KeystoreBridge_set(...) -> jboolean;
//!     fn Java_com_echat_KeystoreBridge_get(...) -> jbyteArray;
//!     fn Java_com_echat_KeystoreBridge_delete(...) -> jboolean;
//! }
//! ```

use async_trait::async_trait;
use echat_core::Result;
use echat_core::ports::keystore::KeystorePort;

/// Android Keystore реализация (stub — требует JNI при сборке под Android).
pub struct AndroidKeystore;

impl AndroidKeystore {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self)
    }
}

#[async_trait]
impl KeystorePort for AndroidKeystore {
    async fn set(&self, service: &str, key: &str, secret: &[u8]) -> Result<()> {
        // TODO: JNI вызов → KeystoreBridge.set(service, key, secret)
        tracing::warn!("AndroidKeystore::set — stub, данные не сохранены");
        Err(core::Error::Keystore(
            "AndroidKeystore не реализован (stub). Требует JNI.".into(),
        ))
    }

    async fn get(&self, service: &str, key: &str) -> Result<Option<Vec<u8>>> {
        // TODO: JNI вызов → KeystoreBridge.get(service, key)
        tracing::warn!("AndroidKeystore::get — stub");
        Ok(None)
    }

    async fn delete(&self, service: &str, key: &str) -> Result<()> {
        // TODO: JNI вызов → KeystoreBridge.delete(service, key)
        tracing::warn!("AndroidKeystore::delete — stub");
        Ok(())
    }
}
