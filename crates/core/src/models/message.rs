use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub account_id: Uuid,
    pub from_email: String,
    pub body: Option<String>,
    pub kind: MessageKind,
    pub status: MessageStatus,
    pub reply_to: Option<Uuid>,
    /// IMAP UID — нужен для удаления с сервера
    pub imap_uid: Option<u32>,
    pub imap_folder: Option<String>,
    pub sent_at: DateTime<Utc>,
    pub received_at: Option<DateTime<Utc>>,
}

impl Message {
    /// Является ли сообщение входящим (от другого пользователя).
    pub fn is_incoming(&self, my_email: &str) -> bool {
        !self.from_email.eq_ignore_ascii_case(my_email)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageKind {
    Text,
    Handshake,
    GroupEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageStatus {
    /// Контакт без приложения — ждём handshake
    Queued,
    /// Отправляется прямо сейчас
    Sending,
    /// Доставлено на SMTP сервер
    Sent,
    /// Появилось на IMAP сервере получателя
    Delivered,
    /// Прочитано
    Read,
    /// Ошибка отправки
    Failed,
}
