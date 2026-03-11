//! # platform
//!
//! Адаптеры — склеивают трейты `core::ports` с конкретными реализациями.
//!
//! ```text
//! core::ports::StoragePort    ←─── StorageAdapter  ←─── crates/storage
//! core::ports::EmailTransport ←─── EmailAdapter    ←─── crates/email
//! core::ports::KeystorePort   ←─── PlatformKeystore ←── OS keychain
//! ```
//!
//! ## Сборка AppState одной функцией
//!
//! ```rust,no_run
//! use platform::build_app_state;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let state = build_app_state(
//!     "alice@mail.ru",
//!     "app_password",
//!     "/data/db.sqlite",
//!     Default::default(),
//! ).await?;
//! # Ok(())
//! # }
//! ```

pub mod email_adapter;
pub mod keystore;
pub mod storage_adapter;

pub use email_adapter::EmailAdapter;
pub use keystore::PlatformKeystore;
pub use storage_adapter::StorageAdapter;

use echat_core::{AppConfig, AppState};

/// Собирает полностью рабочий `AppState` из минимального набора параметров.
///
/// Это единственная точка сборки для `apps/desktop` и `apps/mobile`.
/// Порядок:
/// 1. Открываем SQLite (миграции запускаются автоматически)
/// 2. Подключаемся к IMAP/SMTP
/// 3. Создаём OS keystore
/// 4. Оборачиваем всё в адаптеры
/// 5. Возвращаем `AppState` с готовыми сервисами
pub async fn build_app_state(
    email_addr: &str,
    app_password: &str,
    db_path: &str,
    config: AppConfig,
) -> anyhow::Result<AppState> {
    tracing::info!("Инициализация platform для {}", email_addr);

    // 1. База данных
    let db = storage::Database::open(db_path).await?;
    let storage_adapter = StorageAdapter::new(db);

    // 2. Почтовый клиент
    let provider_config = email::providers::ProviderConfig::detect(email_addr, app_password)
        .ok_or_else(|| anyhow::anyhow!("Неизвестный провайдер для {}", email_addr))?;
    let email_client = email::EmailClient::connect(provider_config).await?;
    let email_adapter = EmailAdapter::new(email_client);

    // 3. OS keystore
    let keystore = PlatformKeystore::new();

    // 4. AppState
    let state = AppState::new(email_adapter, storage_adapter, keystore, config);

    tracing::info!("AppState готов");
    Ok(state)
}
