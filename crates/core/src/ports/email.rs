//! Трейт EmailTransport — абстракция над IMAP/SMTP.
//!
//! Ядро вызывает этот трейт. Реализация живёт в `crates/email`.
//! При тестировании подставляется mock.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::Result;

/// Исходящее письмо — минимальный набор полей, нужных ядру.
#[derive(Debug, Clone)]
pub struct OutgoingEmail {
    pub from: String,
    pub to: Vec<String>,
    pub subject: String,
    pub body: String,
    /// Дополнительные заголовки (X-EChat: 1 и др.)
    pub extra_headers: Vec<(String, String)>,
}

/// Входящее письмо — то что SyncEngine получает с IMAP.
#[derive(Debug, Clone)]
pub struct IncomingEmail {
    pub uid: u32,
    pub folder: String,
    pub from: String,
    pub to: Vec<String>,
    pub subject: String,
    pub body: String,
    pub headers: Vec<(String, String)>,
    pub date: DateTime<Utc>,
}

/// Абстракция над IMAP/SMTP транспортом.
#[async_trait]
pub trait EmailTransport: Send + Sync + 'static {
    /// Отправляет письмо через SMTP и сохраняет копию в папке.
    async fn send(&self, email: OutgoingEmail) -> Result<()>;

    /// Получает новые письма из папки EChat начиная с UID.
    async fn fetch_new(&self, since_uid: Option<u32>) -> Result<Vec<IncomingEmail>>;

    /// Ожидает новых писем через IMAP IDLE.
    /// Возвращает `true` если пришло новое письмо, `false` при таймауте.
    async fn idle_wait(&self) -> Result<bool>;

    /// Удаляет письма с сервера.
    async fn delete_messages(&self, folder: &str, uids: &[u32]) -> Result<()>;

    /// Перемещает письма из одной папки в другую.
    async fn move_messages(&self, from_folder: &str, to_folder: &str, uids: &[u32]) -> Result<()>;

    /// Создаёт папку EChat если её нет.
    async fn ensure_echat_folder(&self) -> Result<()>;
}

/// Тип с динамической диспетчеризацией для хранения в AppState.
pub type DynEmailTransport = Arc<dyn EmailTransport>;
