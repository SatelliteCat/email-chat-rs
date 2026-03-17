//! # storage
//!
//! SQLite-слой персистентности для email-chat.
//!
//! ## Использование
//!
//! ```rust,no_run
//! use storage::Database;
//!
//! # async fn example() -> storage::Result<()> {
//! // Открываем (или создаём) базу данных
//! let db = Database::open("/path/to/db.sqlite").await?;
//!
//! // Репозитории доступны через поля
//! let accounts = db.accounts();
//! let contacts = db.contacts();
//! let conversations = db.conversations();
//! let messages = db.messages();
//! # Ok(())
//! # }
//! ```

pub mod account_repo;
pub mod contact_repo;
pub mod conversation_keys_repo;
pub mod conversation_repo;
pub mod message_repo;
pub mod models;

pub use account_repo::AccountRepo;
pub use contact_repo::ContactRepo;
pub use conversation_keys_repo::ConversationKeyRepo;
pub use conversation_repo::ConversationRepo;
pub use message_repo::MessageRepo;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};
use std::str::FromStr;
use tracing::info;

/// Ошибки крейта storage.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Ошибка базы данных: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error("Ошибка миграции: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    #[error("Запись не найдена: {0}")]
    NotFound(String),

    #[error("Конфликт уникальности: {0}")]
    Conflict(String),

    #[error("Ошибка сериализации: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Центральный объект — пул соединений + доступ к репозиториям.
#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// Открывает существующую БД или создаёт новую, прогоняет миграции.
    ///
    /// `path` — путь к файлу SQLite, например:
    /// - Desktop: `~/.local/share/echat/db.sqlite`
    /// - Android: `/data/data/com.echat/databases/db.sqlite`
    /// - iOS: `<app_documents>/db.sqlite`
    pub async fn open(path: &str) -> Result<Self> {
        info!("Открытие БД: {}", path);

        let options = SqliteConnectOptions::from_str(path)?
            .create_if_missing(true)
            // WAL режим — лучше для конкурентных чтений
            .journal_mode(SqliteJournalMode::Wal)
            // NORMAL — достаточно для наших нужд (не банк)
            .synchronous(SqliteSynchronous::Normal)
            // Внешние ключи по умолчанию выключены в SQLite!
            .foreign_keys(true);

        let pool = SqlitePool::connect_with(options).await?;

        // Применяем все миграции из папки migrations/
        sqlx::migrate!("./migrations").run(&pool).await?;

        info!("БД готова, миграции применены");
        Ok(Self { pool })
    }

    /// In-memory БД для тестов.
    pub async fn open_in_memory() -> Result<Self> {
        let pool = SqlitePool::connect(":memory:").await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    /// Репозиторий аккаунтов.
    pub fn accounts(&self) -> AccountRepo {
        AccountRepo::new(self.pool.clone())
    }

    /// Репозиторий контактов.
    pub fn contacts(&self) -> ContactRepo {
        ContactRepo::new(self.pool.clone())
    }

    /// Репозиторий бесед.
    pub fn conversations(&self) -> ConversationRepo {
        ConversationRepo::new(self.pool.clone())
    }

    /// Репозиторий ключей диалогов.
    pub fn conversation_keys(&self) -> ConversationKeyRepo {
        ConversationKeyRepo::new(self.pool.clone())
    }

    /// Репозиторий сообщений.
    pub fn messages(&self) -> MessageRepo {
        MessageRepo::new(self.pool.clone())
    }

    /// Прямой доступ к пулу — для транзакций spanning нескольких репозиториев.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}
