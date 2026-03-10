//! Общие типы данных — то что пересекает границы модулей.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// UID письма на IMAP сервере.
/// Используется для удаления и дедупликации.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MessageUid(pub u32);

impl std::fmt::Display for MessageUid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Входящее письмо — после получения с IMAP сервера.
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    /// UID на сервере (нужен для удаления)
    pub uid: MessageUid,
    /// Папка где лежит письмо
    pub folder: String,
    /// Адрес отправителя
    pub from: String,
    /// Адреса получателей (To + CC)
    pub to: Vec<String>,
    /// Тема письма
    pub subject: String,
    /// Тело письма (plain text)
    pub body: String,
    /// Заголовки (нужны для детектирования echat-писем)
    pub headers: RawEmailHeaders,
    /// Дата получения
    pub date: DateTime<Utc>,
}

/// Исходящее письмо — передаётся в SmtpConnection.
#[derive(Debug, Clone)]
pub struct OutgoingMessage {
    /// От кого (должен совпадать с аккаунтом)
    pub from: String,
    /// Кому
    pub to: Vec<String>,
    /// Тема
    pub subject: String,
    /// Тело (уже замаскированный base64 payload или invite-текст)
    pub body: String,
    /// Дополнительные заголовки (например X-EChat: 1)
    pub extra_headers: Vec<(String, String)>,
}

/// Заголовки письма — список пар (имя, значение).
#[derive(Debug, Clone, Default)]
pub struct RawEmailHeaders(pub Vec<(String, String)>);

impl RawEmailHeaders {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn as_slice(&self) -> &[(String, String)] {
        &self.0
    }

    /// Конвертирует в формат для disguise::is_echat_message.
    pub fn as_str_pairs(&self) -> Vec<(&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
    }
}
