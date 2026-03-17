//! IMAP клиент.
//!
//! ## Возможности
//!
//! - Подключение через TLS (порт 993)
//! - Получение новых писем из папки EChat (UID-based, без дублей)
//! - IMAP IDLE — push-уведомления о новых письмах без polling
//! - Polling fallback если IDLE не поддерживается
//! - Удаление писем (STORE \Deleted + EXPUNGE)
//! - Создание папок
//!
//! ## IMAP IDLE
//!
//! IDLE держит TCP соединение открытым и сервер присылает EXISTS/EXPUNGE
//! без необходимости периодических запросов. Тайм-аут по RFC 2177 — 29 минут,
//! после чего нужно переподключиться.

use std::time::Duration;

use async_imap::Session;
use chrono::{DateTime, Utc};
use tokio::net::TcpStream;
use tokio_native_tls::TlsStream;
use tracing::{debug, info, warn};

use crate::{
    Error, Result,
    providers::ProviderConfig,
    types::{IncomingMessage, MessageUid, RawEmailHeaders},
};

/// Активное IMAP соединение.
pub struct ImapConnection {
    /// Внутреннее состояние — Mutex для доступа из нескольких задач.
    /// Option потому что IDLE временно забирает владение сессией.
    inner: tokio::sync::Mutex<Option<ImapSession>>,
    config: ProviderConfig,
}

type ImapSession = Session<TlsStream<TcpStream>>;

impl ImapConnection {
    /// Устанавливает TLS соединение и аутентифицируется на IMAP сервере.
    pub async fn connect(config: &ProviderConfig) -> Result<Self> {
        let session = connect_and_login(config).await?;
        Ok(Self {
            inner: tokio::sync::Mutex::new(Some(session)),
            config: config.clone(),
        })
    }

    /// Получает новые письма из папки EChat начиная с UID.
    ///
    /// Если `since_uid` = None — получает все письма в папке.
    pub async fn fetch_new(
        &self,
        config: &ProviderConfig,
        since_uid: Option<MessageUid>,
    ) -> Result<Vec<IncomingMessage>> {
        let mut guard = self.inner.lock().await;
        let session = self.get_or_reconnect(&mut guard, config).await?;

        // Выбираем папку INBOX для получения новых писем
        let folder = "INBOX";
        session
            .select(folder)
            .await
            .map_err(|e| Error::Imap(e.to_string()))?;

        // Строим UID SET: "UID+1:*" или "1:*"
        let uid_set = match since_uid {
            Some(uid) => format!("{}:*", uid.0 + 1),
            None => "1:*".to_string(),
        };

        // Запрашиваем UID SEARCH для получения списка UID
        let uids = session
            .uid_search(format!("UID {}", uid_set))
            .await
            .map_err(|e| Error::Imap(e.to_string()))?;

        tracing::debug!("IMAP UID SEARCH: folder={}, uid_set={}, найдено {} писем", folder, uid_set, uids.len());

        if uids.is_empty() {
            return Ok(vec![]);
        }

        // Формируем UID SET из найденных UID
        let uid_list: Vec<String> = uids.iter().map(|u| u.to_string()).collect();
        let fetch_set = uid_list.join(",");

        // Загружаем письма: заголовки + тело
        let messages_stream = session
            .uid_fetch(&fetch_set, "(UID RFC822.HEADER RFC822.TEXT INTERNALDATE)")
            .await
            .map_err(|e| Error::Imap(e.to_string()))?;

        use futures_util::TryStreamExt;
        let raw_messages: Vec<_> = messages_stream
            .try_collect()
            .await
            .map_err(|e| Error::Imap(e.to_string()))?;

        let folder_name = folder.clone();
        let mut result = Vec::new();
        for msg in raw_messages {
            match parse_imap_message(&msg, &folder_name) {
                Ok(incoming) => result.push(incoming),
                Err(e) => {
                    warn!("Не удалось разобрать письмо uid={:?}: {}", msg.uid, e);
                }
            }
        }

        debug!("Получено {} новых писем из {}", result.len(), folder);
        Ok(result)
    }

    /// Ожидает новых писем через IMAP IDLE.
    ///
    /// Блокирует до прихода EXISTS нотификации или таймаута (29 минут).
    /// Возвращает `true` если пришло новое письмо.
    pub async fn idle_once(&self) -> Result<bool> {
        let mut guard = self.inner.lock().await;
        let mut session = guard.take().ok_or(Error::Disconnected)?;

        // Выбираем INBOX перед IDLE
        session
            .select("INBOX")
            .await
            .map_err(|e| Error::Imap(e.to_string()))?;

        let idle_result = run_idle(session, Duration::from_secs(29 * 60)).await;

        match idle_result {
            Ok((session, got_new)) => {
                *guard = Some(session);
                Ok(got_new)
            }
            Err(e) => {
                // Соединение потеряно — reconnect при следующем запросе
                *guard = None;
                warn!("IDLE завершился с ошибкой: {}. Будет переподключение.", e);
                // Возвращаем true чтобы вызвать fetch (на случай пропущенных писем)
                Ok(true)
            }
        }
    }

    /// Удаляет письма с сервера по UID.
    pub async fn delete_messages(&self, folder: &str, uids: &[MessageUid]) -> Result<()> {
        if uids.is_empty() {
            return Ok(());
        }

        let mut guard = self.inner.lock().await;
        let session = self
            .get_or_reconnect(&mut guard, &self.config.clone())
            .await?;

        session
            .select(folder)
            .await
            .map_err(|e| Error::Imap(e.to_string()))?;

        // Формируем UID SET: "1,5,7,10"
        let uid_set: Vec<String> = uids.iter().map(|u| u.0.to_string()).collect();
        let uid_set_str = uid_set.join(",");

        // Ставим флаг \Deleted
        let _ = session
            .uid_store(&uid_set_str, "+FLAGS (\\Deleted)")
            .await
            .map_err(|e| Error::Imap(e.to_string()))?;

        // Физически удаляем
        let _ = session
            .expunge()
            .await
            .map_err(|e| Error::Imap(e.to_string()))?;

        info!("Удалено {} писем из {}", uids.len(), folder);
        Ok(())
    }

    /// Перемещает письма из одной папки в другую.
    pub async fn move_messages(
        &self,
        from_folder: &str,
        to_folder: &str,
        uids: &[MessageUid],
    ) -> Result<()> {
        if uids.is_empty() {
            return Ok(());
        }

        let mut guard = self.inner.lock().await;
        let session = self
            .get_or_reconnect(&mut guard, &self.config.clone())
            .await?;

        // Выбираем исходную папку
        session
            .select(from_folder)
            .await
            .map_err(|e| Error::Imap(e.to_string()))?;

        // Формируем UID SET
        let uid_set: Vec<String> = uids.iter().map(|u| u.0.to_string()).collect();
        let uid_set_str = uid_set.join(",");

        // Копируем письма в целевую папку
        let _ = session
            .uid_copy(&uid_set_str, to_folder)
            .await
            .map_err(|e| Error::Imap(e.to_string()))?;

        // Удаляем из исходной папки
        let _ = session
            .uid_store(&uid_set_str, "+FLAGS (\\Deleted)")
            .await
            .map_err(|e| Error::Imap(e.to_string()))?;

        let _ = session
            .expunge()
            .await
            .map_err(|e| Error::Imap(e.to_string()))?;

        info!(
            "Перемещено {} писем из {} в {}",
            uids.len(),
            from_folder,
            to_folder
        );
        Ok(())
    }

    /// Создаёт папку если она не существует.
    pub async fn ensure_folder(&self, folder: &str) -> Result<()> {
        let mut guard = self.inner.lock().await;
        let session = self
            .get_or_reconnect(&mut guard, &self.config.clone())
            .await?;

        // Пробуем выбрать папку
        match session.select(folder).await {
            Ok(_) => return Ok(()),
            Err(_) => {
                // Папка не существует — создаём
                session
                    .create(folder)
                    .await
                    .map_err(|e| Error::Imap(e.to_string()))?;
                info!("Создана папка IMAP: {}", folder);
                Ok(())
            }
        }
    }

    /// Сохраняет письмо в указанную папку через IMAP APPEND.
    pub async fn append_message(&self, folder: &str, email_bytes: &[u8]) -> Result<()> {
        let mut guard = self.inner.lock().await;
        let session = self
            .get_or_reconnect(&mut guard, &self.config.clone())
            .await?;

        // Выполняем APPEND
        session
            .append(folder, email_bytes)
            .await
            .map_err(|e| Error::Imap(e.to_string()))?;

        info!("Письмо сохранено в папку {}", folder);
        Ok(())
    }

    // ── Внутренние методы ─────────────────────────────────────────────────────

    /// Получает сессию, при необходимости переподключается.
    async fn get_or_reconnect<'a>(
        &self,
        guard: &'a mut Option<ImapSession>,
        config: &ProviderConfig,
    ) -> Result<&'a mut ImapSession> {
        if guard.is_none() {
            info!("Переподключение к IMAP серверу...");
            let session = connect_and_login(config).await?;
            *guard = Some(session);
        }
        Ok(guard.as_mut().unwrap())
    }
}

// ── Функции подключения ───────────────────────────────────────────────────────

/// Устанавливает TLS соединение и выполняет LOGIN.
async fn connect_and_login(config: &ProviderConfig) -> Result<ImapSession> {
    let host = &config.imap.host;
    let port = config.imap.port;

    info!("IMAP подключение к {}:{}", host, port);

    // TCP подключение
    let tcp = TcpStream::connect(format!("{}:{}", host, port))
        .await
        .map_err(|e| Error::Connect {
            host: host.clone(),
            reason: e.to_string(),
        })?;

    // TLS обёртка
    let tls_connector = native_tls::TlsConnector::new().map_err(|e| Error::Tls(e.to_string()))?;
    let tls = tokio_native_tls::TlsConnector::from(tls_connector);
    let tls_stream = tls
        .connect(host, tcp)
        .await
        .map_err(|e| Error::Tls(e.to_string()))?;

    // IMAP клиент
    let client = async_imap::Client::new(tls_stream);

    // Аутентификация
    let session = client
        .login(&config.email, &config.app_password)
        .await
        .map_err(|(e, _)| {
            let msg = e.to_string();
            if msg.contains("AUTHENTICATIONFAILED")
                || msg.contains("Invalid credentials")
                || msg.contains("authentication failed")
            {
                Error::Auth
            } else {
                Error::Imap(msg)
            }
        })?;

    info!("IMAP аутентификация успешна для {}", config.email);
    Ok(session)
}

/// Запускает IMAP IDLE и ждёт нотификации или таймаута.
async fn run_idle(
    session: ImapSession,
    timeout: Duration,
) -> std::result::Result<(ImapSession, bool), Error> {
    let mut idle = session.idle();

    idle.init().await.map_err(|e| Error::Imap(e.to_string()))?;

    // Ждём с таймаутом
    let (wait_future, _stop_source) = idle.wait();
    let result = tokio::time::timeout(timeout, wait_future).await;

    let got_new = match result {
        Ok(Ok(reason)) => {
            // Проверяем причину — нас интересует EXISTS (новое письмо)
            let s = format!("{:?}", reason);
            s.contains("NewData") || s.contains("EXISTS")
        }
        Ok(Err(e)) => {
            return Err(Error::Imap(e.to_string()));
        }
        Err(_timeout) => {
            // Истёк таймаут — нормально, просто переподключаемся
            false
        }
    };

    // Восстанавливаем сессию из IDLE
    let session = idle.done().await.map_err(|e| Error::Imap(e.to_string()))?;

    Ok((session, got_new))
}

// ── Парсинг ───────────────────────────────────────────────────────────────────

/// Разбирает сырое IMAP сообщение в IncomingMessage.
fn parse_imap_message(msg: &async_imap::types::Fetch, folder: &str) -> Result<IncomingMessage> {
    let uid = MessageUid(msg.uid.ok_or_else(|| Error::Parse("нет UID".into()))?);

    // Заголовки
    let header_bytes = msg
        .header()
        .ok_or_else(|| Error::Parse("нет заголовков".into()))?;
    let header_str = std::str::from_utf8(header_bytes).map_err(|e| Error::Parse(e.to_string()))?;

    let (headers, from, to, subject) = parse_headers(header_str)?;

    // Тело письма — используем mail-parser для корректного декодирования
    // Content-Transfer-Encoding (base64, quoted-printable)
    let body_bytes = msg.text().unwrap_or(b"");
    let body = decode_body(body_bytes, header_str);

    // Дата
    let date = msg
        .internal_date()
        .map(|d| DateTime::from_timestamp(d.timestamp(), 0).unwrap_or_else(Utc::now))
        .unwrap_or_else(Utc::now);

    Ok(IncomingMessage {
        uid,
        folder: folder.to_string(),
        from,
        to,
        subject,
        body,
        headers,
        date,
    })
}

/// Декодирует тело письма с учётом Content-Transfer-Encoding.
fn decode_body(body_bytes: &[u8], headers_str: &str) -> String {
    // Пробуем распарсить полное письмо для доступа к MIME-частям
    let full_message = [headers_str.as_bytes(), b"\r\n\r\n", body_bytes].concat();
    let parser = mail_parser::MessageParser::default();

    if let Some(parsed) = parser.parse(&full_message[..]) {
        // Пытаемся получить текстовую часть с автоматическим декодированием
        if let Some(body_text) = parsed.body_text(0) {
            return body_text.to_string();
        }
    }

    // Fallback: возвращаем тело как есть, удаляя переносы строк
    // (для base64 это нормально)
    String::from_utf8_lossy(body_bytes).to_string()
}

/// Разбирает строку заголовков RFC 2822.
fn parse_headers(raw: &str) -> Result<(RawEmailHeaders, String, Vec<String>, String)> {
    // Используем mail-parser для надёжного парсинга заголовков
    let parser = mail_parser::MessageParser::default();
    let parsed = parser
        .parse(raw.as_bytes())
        .ok_or_else(|| Error::Parse("mail-parser: не удалось распарсить письмо".into()))?;

    let mut headers = RawEmailHeaders::default();

    // Извлекаем все заголовки
    for header in parsed.headers() {
        let name = header.name.to_string();
        let value = header.value.as_text().unwrap_or("").to_string();
        headers.0.push((name, value));
    }

    // Извлекаем From
    let from = parsed
        .from()
        .and_then(|m| m.first())
        .and_then(|m| m.address().map(|s| s.to_string()))
        .unwrap_or_default();

    // Извлекаем To
    let mut to = Vec::new();
    if let Some(to_list) = parsed.to() {
        for mailbox in to_list.iter() {
            if let Some(addr) = mailbox.address() {
                to.push(addr.to_string());
            }
        }
    }

    // Извлекаем Subject
    let subject = parsed.subject().unwrap_or("").to_string();

    Ok((headers, from, to, subject))
}

/// Извлекает email адрес из строки типа "Name <email@domain.com>" или "email@domain.com".
fn extract_email_address(s: &str) -> String {
    if let Some(start) = s.find('<') {
        if let Some(end) = s.find('>') {
            return s[start + 1..end].trim().to_string();
        }
    }
    s.trim().to_string()
}

/// Декодирует MIME encoded-word (=?UTF-8?B?...?= или =?UTF-8?Q?...?=).
/// Упрощённая реализация — для production стоит использовать mail-parser.
fn decode_mime_header(s: &str) -> String {
    if !s.contains("=?") {
        return s.to_string();
    }
    // Упрощённо: возвращаем как есть, mail-parser обработает полностью
    s.to_string()
}
