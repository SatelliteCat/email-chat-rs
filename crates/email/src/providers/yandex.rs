//! Конфигурация Яндекс Почты
//!
//! ## Требования Яндекс
//!
//! 1. Включить IMAP: Настройки → Почтовые клиенты → С сервера imap.yandex.ru
//! 2. Создать пароль приложения: Аккаунт → Безопасность → Пароли приложений
//!    Тип: "Почта"
//!
//! ## Параметры
//!
//! | Протокол | Хост               | Порт | TLS   |
//! |----------|--------------------|------|-------|
//! | IMAP     | imap.yandex.ru     | 993  | SSL   |
//! | SMTP     | smtp.yandex.ru     | 465  | SSL   |
//!
//! ## Домены Яндекс
//!
//! yandex.ru, ya.ru, yandex.com, yandex.kz, yandex.by, yandex.ua —
//! все используют одни серверы.

use crate::providers::{ImapConfig, Provider, ProviderConfig, SmtpConfig, TlsMode};

pub fn config(email: String, app_password: String) -> ProviderConfig {
    ProviderConfig {
        provider: Provider::Yandex,
        email,
        app_password,
        imap: ImapConfig {
            host: "imap.yandex.ru".to_string(),
            port: 993,
            tls: TlsMode::Tls,
        },
        smtp: SmtpConfig {
            host: "smtp.yandex.ru".to_string(),
            port: 465,
            tls: TlsMode::Tls,
        },
        echat_folder: "INBOX".to_string(),
    }
}
