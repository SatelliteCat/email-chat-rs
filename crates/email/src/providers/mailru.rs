//! Конфигурация Mail.ru
//!
//! ## Требования Mail.ru
//!
//! 1. Включить IMAP в настройках почты: Настройки → Почтовые программы
//! 2. Создать пароль приложения: Аккаунт → Безопасность → Пароли для внешних приложений
//!    (основной пароль НЕ принимается для IMAP/SMTP)
//!
//! ## Параметры
//!
//! | Протокол | Хост              | Порт | TLS   |
//! |----------|-------------------|------|-------|
//! | IMAP     | imap.mail.ru      | 993  | SSL   |
//! | SMTP     | smtp.mail.ru      | 465  | SSL   |
//!
//! ## Домены Mail.ru Group
//!
//! Все домены используют одни серверы: mail.ru, inbox.ru, list.ru, bk.ru

use crate::providers::{ImapConfig, Provider, ProviderConfig, SmtpConfig, TlsMode};

pub fn config(email: String, app_password: String) -> ProviderConfig {
    ProviderConfig {
        provider: Provider::MailRu,
        email,
        app_password,
        imap: ImapConfig {
            host: "imap.mail.ru".to_string(),
            port: 993,
            tls: TlsMode::Tls,
        },
        smtp: SmtpConfig {
            host: "smtp.mail.ru".to_string(),
            port: 465,
            tls: TlsMode::Tls,
        },
        echat_folder: "INBOX".to_string(),
    }
}
