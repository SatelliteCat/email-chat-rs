//! # core
//!
//! Бизнес-логика email-chat. Не зависит от UI, платформы и
//! конкретных реализаций email/storage — только от трейтов в `ports/`.
//!
//! ## Сборка AppState
//!
//! ```rust,no_run
//! use core::{AppState, AppConfig};
//! use std::sync::Arc;
//!
//! // Конкретные реализации инжектируются снаружи:
//! // let email_transport = Arc::new(email::EmailClient::connect(...).await?);
//! // let storage = Arc::new(StorageAdapter::new(db));
//! // let keystore = Arc::new(platform::keystore::OsKeystore::new());
//!
//! // let state = AppState::new(email_transport, storage, keystore, config);
//! // state.sync_engine.start();
//! ```

pub mod events;
pub mod models;
pub mod ports;
pub mod services;
pub mod sync;

pub use events::{ChatEvent, EventBus};

// Нужен async_trait для портов
extern crate async_trait;

/// Ошибки ядра приложения.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Не найдено: {0}")]
    NotFound(String),

    #[error("Уже существует: {0}")]
    AlreadyExists(String),

    #[error("Недостаточно прав: {0}")]
    Forbidden(String),

    #[error("Некорректное состояние: {0}")]
    InvalidState(String),

    #[error("Ошибка шифрования: {0}")]
    Encryption(String),

    #[error("Ошибка расшифровки")]
    Decrypt,

    #[error("Ошибка транспорта: {0}")]
    Transport(String),

    #[error("Ошибка хранилища: {0}")]
    Storage(String),

    #[error("Ошибка keystore: {0}")]
    Keystore(String),

    #[error("Внутренняя ошибка: {0}")]
    Internal(String),

    #[error("Ошибка БД: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error("Ошибка сериализации: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Конфликт: {0}")]
    Conflict(String),
}

pub type Result<T> = std::result::Result<T, Error>;

// Конвертации из ошибок нижних слоёв
impl From<encryption::Error> for Error {
    fn from(e: encryption::Error) -> Self {
        Error::Encryption(e.to_string())
    }
}

/// Конфигурация приложения.
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// URL для скачивания приложения (вставляется в invite-письма)
    pub app_download_url: String,
    /// Ёмкость буфера EventBus
    pub event_bus_capacity: usize,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            app_download_url: "https://echat.app".to_string(),
            event_bus_capacity: 256,
        }
    }
}

/// Центральный объект приложения — держит все сервисы.
///
/// Создаётся один раз в `apps/desktop` или `apps/mobile`,
/// передаётся во все части UI.
pub struct AppState {
    pub account_service: services::account::AccountService,
    pub contact_service: services::contacts::ContactService,
    pub chat_service: services::chat::ChatService,
    pub group_service: services::group::GroupService,
    pub events: EventBus,
    /// Email транспорт для SyncEngine
    email: ports::email::DynEmailTransport,
    /// Хранилище для SyncEngine
    storage: ports::storage::DynStorage,
    /// Keystore для SyncEngine
    keystore: ports::keystore::DynKeystore,
}

impl AppState {
    /// Собирает AppState из конкретных реализаций портов.
    pub fn new(
        email: ports::email::DynEmailTransport,
        storage: ports::storage::DynStorage,
        keystore: ports::keystore::DynKeystore,
        config: AppConfig,
    ) -> Self {
        let events = EventBus::new(config.event_bus_capacity);

        let account_svc = services::account::AccountService::new(storage.clone(), keystore.clone());

        let contact_svc = services::contacts::ContactService::new(
            storage.clone(),
            email.clone(),
            services::account::AccountService::new(storage.clone(), keystore.clone()),
            events.clone(),
        );

        let chat_svc = services::chat::ChatService::new(
            storage.clone(),
            email.clone(),
            services::account::AccountService::new(storage.clone(), keystore.clone()),
            events.clone(),
            config.app_download_url.clone(),
        );

        let group_svc = services::group::GroupService::new(
            storage.clone(),
            email.clone(),
            services::account::AccountService::new(storage.clone(), keystore.clone()),
            events.clone(),
        );

        Self {
            account_service: account_svc,
            contact_service: contact_svc,
            chat_service: chat_svc,
            group_service: group_svc,
            events,
            email,
            storage,
            keystore,
        }
    }

    /// Запускает SyncEngine для аккаунта в фоновой задаче.
    /// Возвращает sender для команд и async блок который нужно заспавнить.
    ///
    /// Async блок нужно запустить через runtime.spawn() потому что
    /// tokio::spawn должен вызываться из контекста runtime.
    pub fn spawn_sync(
        &self,
        account_id: uuid::Uuid,
    ) -> (
        tokio::sync::mpsc::Sender<sync::engine::SyncCommand>,
        impl std::future::Future<Output = ()> + Send + 'static,
    ) {
        let config = AppConfig::default();
        let email = self.email.clone();
        let storage = self.storage.clone();
        let keystore = self.keystore.clone();
        let events = self.events.clone();

        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(32);

        let account_svc = services::account::AccountService::new(storage.clone(), keystore.clone());
        let contact_svc = services::contacts::ContactService::new(
            storage.clone(),
            email.clone(),
            services::account::AccountService::new(storage.clone(), keystore.clone()),
            events.clone(),
        );
        let chat_svc = services::chat::ChatService::new(
            storage.clone(),
            email.clone(),
            services::account::AccountService::new(storage.clone(), keystore.clone()),
            events.clone(),
            config.app_download_url.clone(),
        );

        let future = async move {
            sync::engine::run_sync_loop(
                account_id,
                email,
                storage,
                account_svc,
                contact_svc,
                chat_svc,
                events,
                cmd_rx,
            )
            .await;
        };

        (cmd_tx, future)
    }
}
