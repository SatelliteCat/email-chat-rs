//! SMTP клиент для отправки писем.
//!
//! Использует `lettre` с TLS (порт 465, implicit TLS).
//! Соединение устанавливается заново для каждой отправки —
//! это надёжнее чем держать persistent connection (серверы часто
//! разрывают idle SMTP соединения через 5-10 минут).

use lettre::{
    Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{
        Mailbox, SinglePart,
        header::{ContentTransferEncoding, ContentType, HeaderName, HeaderValue},
    },
    transport::smtp::{
        authentication::Credentials,
        client::{Tls, TlsParameters},
    },
};
use tracing::info;

use crate::{
    Error, Result,
    providers::{ProviderConfig, TlsMode},
    types::OutgoingMessage,
};

/// SMTP соединение.
///
/// Внутри хранит конфигурацию для построения транспорта при отправке.
/// Lettre сам управляет connection pool'ом.
pub struct SmtpConnection {
    _config: ProviderConfig,
}

impl SmtpConnection {
    /// Проверяет конфигурацию и подготавливает транспорт.
    /// Реальное TCP соединение устанавливается при первой отправке.
    pub async fn connect(config: &ProviderConfig) -> Result<Self> {
        // Проверяем что можем построить транспорт
        build_transport(config)?;
        info!("SMTP транспорт подготовлен для {}", config.smtp.host);
        Ok(Self {
            _config: config.clone(),
        })
    }

    /// Отправляет письмо и возвращает его сырые байты.
    pub async fn send(&self, msg: &OutgoingMessage, config: &ProviderConfig) -> Result<Vec<u8>> {
        let transport = build_transport(config)?;
        let email = build_email(msg, config)?;

        transport.send(email.clone()).await.map_err(|e| {
            let s = e.to_string();
            if s.contains("535") || s.contains("Authentication") {
                Error::Auth
            } else {
                Error::Smtp(s)
            }
        })?;

        info!("Письмо отправлено: {} → {:?}", msg.from, msg.to);

        // Возвращаем сырые байты письма для сохранения в папку
        Ok(email.formatted())
    }
}

// ── Внутренние функции ────────────────────────────────────────────────────────

/// Строит SMTP транспорт из конфигурации.
fn build_transport(config: &ProviderConfig) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
    let creds = Credentials::new(config.email.clone(), config.app_password.clone());

    let tls_params =
        TlsParameters::new(config.smtp.host.clone()).map_err(|e| Error::Tls(e.to_string()))?;

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
fn build_email(msg: &OutgoingMessage, _config: &ProviderConfig) -> Result<Message> {
    // Парсим from адрес
    let from_addr: Address = msg
        .from
        .parse()
        .map_err(|e: lettre::address::AddressError| {
            Error::Smtp(format!("неверный from адрес: {}", e))
        })?;
    let from_mailbox = Mailbox::new(None, from_addr);

    // Строим сообщение
    let mut builder = Message::builder().from(from_mailbox).subject(&msg.subject);

    // Добавляем получателей
    for to_addr in &msg.to {
        let addr: Address = to_addr
            .parse()
            .map_err(|e: lettre::address::AddressError| {
                Error::Smtp(format!("неверный to адрес '{}': {}", to_addr, e))
            })?;
        builder = builder.to(Mailbox::new(None, addr));
    }

    // Добавляем кастомные заголовки (X-EChat: 1 и др.)
    for (name, value) in &msg.extra_headers {
        builder = builder.raw_header(HeaderValue::new(
            HeaderName::new_from_ascii(name.clone()).unwrap(),
            value.clone(),
        ));
    }

    // Явно указываем Content-Transfer-Encoding: base64
    // Тело письма уже содержит base64-данные, поэтому quoted-printable
    // испортит их (добавит = в концах строк)
    let email = builder
        .singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_PLAIN)
                .header(ContentTransferEncoding::Base64)
                .body(msg.body.clone()),
        )
        .map_err(|e| Error::Smtp(e.to_string()))?;

    Ok(email)
}
