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
    // 1. База данных
    let db = storage::Database::open(db_path).await?;
    let storage_adapter = StorageAdapter::new(db.clone());

    // 2. Почтовый клиент
    // Сначала определяем конфиг по умолчанию
    let mut provider_config = email::providers::ProviderConfig::detect(email_addr, app_password)
        .ok_or_else(|| anyhow::anyhow!("Неизвестный провайдер для {}", email_addr))?;

    // Пробуем загрузить аккаунт, чтобы получить сохранённую папку
    if let Ok(account) = db.accounts().get_by_email(email_addr).await {
        provider_config.echat_folder = account.echat_folder;
    }

    let email_client = email::EmailClient::connect(provider_config).await?;
    let email_adapter = EmailAdapter::new(email_client);

    // 3. OS keystore
    let keystore = PlatformKeystore::new();

    // 4. AppState
    let state = AppState::new(email_adapter, storage_adapter, keystore, config);

    Ok(state)
}

/// Восстанавливает последнюю сессию из сохранённых данных.
///
/// Проверяет базу данных на наличие сохранённых аккаунтов,
/// загружает последний использованный аккаунт и восстанавливает
/// credentials из OS keystore.
///
/// Возвращает None если нет сохранённых аккаунтов или не удалось
/// восстановить credentials.
pub async fn restore_last_session(
    db_path: &str,
    config: AppConfig,
) -> anyhow::Result<Option<(AppState, echat_core::models::account::Account)>> {
    // 1. Открываем базу данных
    let db = storage::Database::open(db_path).await?;
    let storage_adapter = StorageAdapter::new(db.clone());
    let keystore = PlatformKeystore::new();

    // 2. Создаём временный AccountService для работы с keystore
    let account_svc =
        echat_core::services::account::AccountService::new(storage_adapter.clone(), keystore);

    // 3. Получаем список всех аккаунтов
    let accounts = match account_svc.list_accounts().await {
        Ok(accs) => accs,
        Err(e) => {
            tracing::warn!("Не удалось загрузить аккаунты: {}", e);
            return Ok(None);
        }
    };

    if accounts.is_empty() {
        tracing::info!("Нет сохранённых аккаунтов");
        return Ok(None);
    }

    // 4. Берём последний аккаунт (предполагаем что он последний в списке)
    let account = accounts.last().ok_or_else(|| anyhow::anyhow!("Пустой список аккаунтов"))?;

    // 5. Восстанавливаем app_password из keystore
    tracing::info!("Пытаемся восстановить пароль для {}", account.email);
    let app_password = match account_svc.get_app_password(&account.email).await {
        Ok(pwd) => {
            tracing::info!("Пароль успешно восстановлен для {}", account.email);
            pwd
        }
        Err(e) => {
            tracing::warn!("Не удалось восстановить пароль для {}: {}", account.email, e);
            return Ok(None);
        }
    };

    // 6. Убеждаемся что identity keypair существует
    account_svc
        .load_or_create_keypair(account.id)
        .await?;

    // 7. Строим полноценный AppState
    let state = build_app_state(&account.email, &app_password, db_path, config).await?;

    tracing::info!("Восстановлена сессия для аккаунта {}", account.email);
    Ok(Some((state, account.clone())))
}
