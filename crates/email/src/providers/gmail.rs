//! Конфигурация Gmail
//!
//! ## Требования Gmail
//!
//! 1. Включить IMAP: Настройки → Все настройки → Пересылка и POP/IMAP → Доступ по протоколу IMAP
//! 2. Использовать пароль приложения (не основной пароль):
//!    - Аккаунт Google → Безопасность → Двухэтапная аутентификация
//!    - Пароли приложений → Выбрать приложение "Почта" и устройство
//!    - Скопировать сгенерированный 16-значный пароль
//!
//! ## Параметры
//!
//! | Протокол | Хост              | Порт | TLS   |
//! |----------|-------------------|------|-------|
//! | IMAP     | imap.gmail.com    | 993  | SSL   |
//! | SMTP     | smtp.gmail.com    | 465  | SSL   |
//!
//! ## Примечания
//!
//! - Gmail требует включенную двухэтапную аутентификацию для использования паролей приложений
//! - Альтернативно можно использовать OAuth 2.0 (не реализовано в данном модуле)

use crate::providers::{ImapConfig, Provider, ProviderConfig, SmtpConfig, TlsMode};

pub fn config(email: String, app_password: String) -> ProviderConfig {
    ProviderConfig {
        provider: Provider::Gmail,
        email,
        app_password,
        imap: ImapConfig {
            host: "imap.gmail.com".to_string(),
            port: 993,
            tls: TlsMode::Tls,
        },
        smtp: SmtpConfig {
            host: "smtp.gmail.com".to_string(),
            port: 465,
            tls: TlsMode::Tls,
        },
        echat_folder: "INBOX".to_string(),
    }
}
