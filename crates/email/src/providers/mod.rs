//! Конфигурации почтовых провайдеров.
//!
//! Каждый провайдер — это набор параметров подключения:
//! IMAP хост/порт, SMTP хост/порт, специфические quirks.

pub mod mailru;
pub mod yandex;

use serde::{Deserialize, Serialize};

/// Параметры IMAP подключения.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub tls: TlsMode,
}

/// Параметры SMTP подключения.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub tls: TlsMode,
}

/// Режим TLS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TlsMode {
    /// Сразу TLS (порты 993/465)
    Tls,
    /// STARTTLS (порт 587)
    StartTls,
}

/// Поддерживаемые провайдеры.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Provider {
    MailRu,
    Yandex,
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Provider::MailRu => write!(f, "Mail.ru"),
            Provider::Yandex => write!(f, "Яндекс"),
        }
    }
}

/// Полная конфигурация аккаунта — всё что нужно для подключения.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub provider: Provider,
    /// Email адрес пользователя
    pub email: String,
    /// Пароль приложения (НЕ основной пароль от аккаунта)
    /// Хранится в памяти только на время сессии.
    /// В постоянном хранилище — через KeyStore.
    pub app_password: String,
    pub imap: ImapConfig,
    pub smtp: SmtpConfig,
    /// Папка для echat-сообщений на IMAP сервере.
    /// По умолчанию: "EChat" (создаётся автоматически).
    pub echat_folder: String,
}

impl ProviderConfig {
    /// Создаёт конфигурацию для Mail.ru.
    pub fn mailru(email: impl Into<String>, app_password: impl Into<String>) -> Self {
        mailru::config(email.into(), app_password.into())
    }

    /// Создаёт конфигурацию для Яндекс.
    pub fn yandex(email: impl Into<String>, app_password: impl Into<String>) -> Self {
        yandex::config(email.into(), app_password.into())
    }

    /// Определяет провайдера по домену email.
    pub fn detect(email: impl Into<String>, app_password: impl Into<String>) -> Option<Self> {
        let email = email.into();
        let password = app_password.into();
        let domain = email.split('@').nth(1)?;

        match domain {
            "mail.ru" | "inbox.ru" | "list.ru" | "bk.ru" => {
                Some(Self::mailru(email, password))
            }
            "yandex.ru" | "ya.ru" | "yandex.com" | "yandex.kz"
            | "yandex.by" | "yandex.ua" => {
                Some(Self::yandex(email, password))
            }
            _ => None,
        }
    }
}
