//! SMTP клиент для отправки писем.
//!
//! Использует `lettre` с TLS (порт 465, implicit TLS).
//! Соединение устанавливается заново для каждой отправки —
//! это надёжнее чем держать persistent connection (серверы часто
//! разрывают idle SMTP соединения через 5-10 минут).

use lettre::{
    message::{header::ContentType, Mailbox, MultiPart, SinglePart},
    transport::smtp::{
        authentication::Credentials,
        client::{Tls, TlsParameters},
    },
    Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
};
use tracing::info;

use crate::{
    providers::{ProviderConfig, TlsMode},
    types::OutgoingMessage,
    Error, Result,
};

/// SMTP соединение.
///
/// Внутри хранит конфигурацию для построения транспорта при отправке.
/// Lettre сам управляет connection pool'ом.
pub struct SmtpConnection {
    config: ProviderConfig,
}

impl SmtpConnection {
    /// Проверяет конфигурацию и подготавливает транспорт.
    /// Реальное TCP соединение устанавливается при первой отправке.
    pub async fn connect(config: &ProviderConfig) -> Result<Self> {
        // Проверяем что можем построить транспорт
        build_transport(config)?;
        info!(
            "SMTP транспорт подготовлен для {}",
            config.smtp.host
        );
        Ok(Self { config: config.clone() })
    }

    /// Отправляет письмо.
    pub async fn send(&self, msg: &OutgoingMessage, config: &ProviderConfig) -> Result<()> {
        let transport = build_transport(config)?;
        let email = build_email(msg, config)?;

        transport
            .send(email)
            .await
            .map_err(|e| {
                let s = e.to_string();
                if s.contains("535") || s.contains("Authentication") {
                    Error::Auth
                } else {
                    Error::Smtp(s)
                }
            })?;

        info!(
            "Письмо отправлено: {} → {:?}",
            msg.from,
            msg.to
        );
        Ok(())
    }
}

// ── Внутренние функции ────────────────────────────────────────────────────────

/// Строит SMTP транспорт из конфигурации.
fn build_transport(
    config: &ProviderConfig,
) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
    let creds = Credentials::new(
        config.email.clone(),
        config.app_password.clone(),
    );

    let tls_params = TlsParameters::new(config.smtp.host.clone())
        .map_err(|e| Error::Tls(e.to_string()))?;

    let transport = match config.smtp.tls {
        TlsMode::Tls => {
            // Implicit TLS (порт 465) — сразу TLS без STARTTLS
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.smtp.host)
                .port(config.smtp.port)
                .tls(Tls::Wrapper(tls_params))
                .credentials(creds)
                .build()
        }
        TlsMode::StartTls => {
            // STARTTLS (порт 587)
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.smtp.host)
                .port(config.smtp.port)
                .tls(Tls::Required(tls_params))
                .credentials(creds)
                .build()
        }
    };

    Ok(transport)
}

/// Строит lettre Message из OutgoingMessage.
fn build_email(
    msg: &OutgoingMessage,
    _config: &ProviderConfig,
) -> Result<Message> {
    // Парсим from адрес
    let from_addr: Address = msg
        .from
        .parse()
        .map_err(|e: lettre::address::AddressError| {
            Error::Smtp(format!("неверный from адрес: {}", e))
        })?;
    let from_mailbox = Mailbox::new(None, from_addr);

    // Строим сообщение
    let mut builder = Message::builder()
        .from(from_mailbox)
        .subject(&msg.subject);

    // Добавляем получателей
    for to_addr in &msg.to {
        let addr: Address =
            to_addr
                .parse()
                .map_err(|e: lettre::address::AddressError| {
                    Error::Smtp(format!("неверный to адрес '{}': {}", to_addr, e))
                })?;
        builder = builder.to(Mailbox::new(None, addr));
    }

    // Добавляем кастомные заголовки (X-EChat: 1 и др.)
    for (name, value) in &msg.extra_headers {
        // lettre не поддерживает произвольные заголовки напрямую через builder,
        // поэтому используем костыль через subject + добавление в body
        // TODO: когда lettre добавит поддержку — использовать нативно
        // Пока X-EChat кодируем как часть тела (disguise обрабатывает)
        let _ = (name, value); // используется в codec.rs
    }

    let email = builder
        .body(msg.body.clone())
        .map_err(|e| Error::Smtp(e.to_string()))?;

    Ok(email)
}
