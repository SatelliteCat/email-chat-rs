//! # email
//!
//! IMAP/SMTP транспортный слой для email-chat.
//!
//! ## Что делает этот крейт
//!
//! - Подключается к почтовым серверам (Mail.ru, Яндекс) через TLS
//! - Отправляет зашифрованные письма через SMTP
//! - Получает письма через IMAP (IDLE push + polling fallback)
//! - Удаляет письма с сервера (для приватности при удалении чата)
//! - Конвертирует email-сообщения ↔ внутренние типы приложения
//!
//! ## Архитектура
//!
//! ```text
//! ┌─────────────────────────────────────────────┐
//! │              EmailClient                    │ ← основной фасад
//! ├──────────────────┬──────────────────────────┤
//! │   ImapConnection │   SmtpConnection         │
//! ├──────────────────┴──────────────────────────┤
//! │              ProviderConfig                 │ ← Mail.ru / Яндекс
//! └─────────────────────────────────────────────┘
//! ```
//!
//! ## Пример
//!
//! ```rust,no_run
//! use email::{EmailClient, providers::ProviderConfig};
//!
//! # async fn example() -> email::Result<()> {
//! let config = ProviderConfig::mailru("user@mail.ru", "app_password");
//! let client = EmailClient::connect(config).await?;
//!
//! // Получить новые сообщения (None = все новые, Some(uid) = начиная с UID)
//! let messages = client.fetch_new_messages(None).await?;
//!
//! // Отправить сообщение
//! client.send_message(todo!()).await?;
//! # Ok(())
//! # }
//! ```

pub mod codec;
pub mod imap;
pub mod providers;
pub mod smtp;
pub mod types;

pub use types::{IncomingMessage, MessageUid, OutgoingMessage, RawEmailHeaders};

/// Ошибки крейта email.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Ошибка IMAP: {0}")]
    Imap(String),

    #[error("Ошибка SMTP: {0}")]
    Smtp(String),

    #[error("Ошибка TLS: {0}")]
    Tls(String),

    #[error("Ошибка аутентификации: неверный логин или пароль")]
    Auth,

    #[error("Ошибка подключения к {host}: {reason}")]
    Connect { host: String, reason: String },

    #[error("Соединение разорвано")]
    Disconnected,

    #[error("Папка не найдена: {0}")]
    FolderNotFound(String),

    #[error("Ошибка разбора письма: {0}")]
    Parse(String),

    #[error("Ошибка кодека: {0}")]
    Codec(String),

    #[error("Таймаут операции")]
    Timeout,

    #[error("Неизвестная ошибка: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

// ── Главный фасад ─────────────────────────────────────────────────────────────

/// Единая точка входа для работы с почтовым сервером.
///
/// Содержит IMAP и SMTP соединения и предоставляет высокоуровневый API
/// который понимает только типы этого крейта, без деталей протоколов.
pub struct EmailClient {
    pub(crate) imap: imap::ImapConnection,
    pub(crate) smtp: smtp::SmtpConnection,
    pub(crate) config: providers::ProviderConfig,
}

impl EmailClient {
    /// Создаёт клиент и устанавливает соединения с IMAP и SMTP серверами.
    pub async fn connect(config: providers::ProviderConfig) -> Result<Self> {
        tracing::info!("Подключение к {}", config.imap.host);

        let imap = imap::ImapConnection::connect(&config).await?;
        let smtp = smtp::SmtpConnection::connect(&config).await?;

        Ok(Self { imap, smtp, config })
    }

    /// Отправляет исходящее сообщение и сохраняет копию в папку.
    pub async fn send_message(&self, msg: OutgoingMessage) -> Result<()> {
        // Отправляем через SMTP и получаем сырые байты
        let email_bytes = self.smtp.send(&msg, &self.config).await?;

        // Убеждаемся что папка существует перед сохранением
        self.ensure_echat_folder().await?;

        // Сохраняем копию в папку EChat через IMAP APPEND
        self.imap
            .append_message(&self.config.echat_folder, &email_bytes)
            .await?;

        Ok(())
    }

    /// Получает новые сообщения из папки echat начиная с UID.
    pub async fn fetch_new_messages(
        &self,
        since_uid: Option<MessageUid>,
    ) -> Result<Vec<IncomingMessage>> {
        self.imap.fetch_new(&self.config, since_uid).await
    }

    /// Запускает IMAP IDLE — блокирует до прихода нового письма
    /// или истечения таймаута (обычно 29 минут по RFC).
    ///
    /// Возвращает `true` если пришло новое письмо, `false` при таймауте.
    pub async fn idle_wait(&self) -> Result<bool> {
        self.imap.idle_once().await
    }

    /// Удаляет письма с сервера по UID.
    pub async fn delete_messages(&self, folder: &str, uids: &[MessageUid]) -> Result<()> {
        self.imap.delete_messages(folder, uids).await
    }

    /// Перемещает письма из одной папки в другую.
    pub async fn move_messages(
        &self,
        from_folder: &str,
        to_folder: &str,
        uids: &[u32],
    ) -> Result<()> {
        let uid_objs: Vec<MessageUid> = uids.iter().map(|u| MessageUid(*u)).collect();
        self.imap
            .move_messages(from_folder, to_folder, &uid_objs)
            .await
    }

    /// Убеждается что папка для echat сообщений существует, создаёт если нет.
    pub async fn ensure_echat_folder(&self) -> Result<()> {
        self.imap.ensure_folder(&self.config.echat_folder).await
    }

    /// Возвращает конфигурацию провайдера.
    pub fn config(&self) -> &providers::ProviderConfig {
        &self.config
    }
}
