//! Маскировка email-заголовков.
//!
//! Письма приложения должны выглядеть как обычная переписка
//! для стороннего наблюдателя (почтовый провайдер, обычный клиент).
//!
//! ## Что маскируется
//! - `Subject` — случайная безобидная тема из заготовленного набора
//! - `Content-Type` — `text/plain` (не `application/octet-stream`)
//! - Тело — выглядит как длинный base64 (похоже на вложение в тексте)
//!
//! ## Что НЕ маскируется (пока)
//! - `From` / `To` — провайдер видит адресатов в любом случае (metadata)
//! - `Date` — обычный timestamp
//!
//! Единственный наш заголовок: `X-EChat: 1` — по нему SyncEngine
//! быстро фильтрует входящие без сканирования тела.

use rand::{seq::SliceRandom, thread_rng};

/// Тип содержимого письма — для правильного форматирования.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyKind {
    /// Обычное зашифрованное сообщение
    EncryptedMessage,
    /// Handshake (обмен ключами)
    Handshake,
    /// Приглашение для пользователя без приложения
    Invite { app_url: String },
}

/// Параметры сформированного письма.
#[derive(Debug, Clone)]
pub struct DisguisedEmail {
    pub subject: String,
    pub body: String,
    /// Заголовки для добавления (помимо стандартных From/To/Date)
    pub extra_headers: Vec<(String, String)>,
}

/// Генерирует тему письма — случайная из заготовленного набора.
pub fn random_subject() -> String {
    let subjects = [
        // Рабочие
        "Re: документы",
        "Re: встреча",
        "Fwd: материалы",
        "Re: вопрос",
        "Fwd: информация",
        "Re: проект",
        // Личные
        "Re: как дела",
        "Fwd: фотографии",
        "Re: планы на выходные",
        "Fwd: интересная статья",
        "Re: договорились",
        "Re: увидимся",
        // Нейтральные
        "Re: ок",
        "Fwd: посмотри",
        "Re:",
        "Fwd:",
    ];

    let mut rng = thread_rng();
    subjects.choose(&mut rng).unwrap_or(&"Re:").to_string()
}

/// Формирует готовое письмо для отправки.
pub fn build_email(body_base64: &str, kind: BodyKind) -> DisguisedEmail {
    let subject = random_subject();

    let body = match &kind {
        BodyKind::EncryptedMessage | BodyKind::Handshake => {
            // Просто base64-блоб — выглядит как случайный мусор
            body_base64.to_string()
        }
        BodyKind::Invite { app_url } => {
            // Читаемый текст + зашифрованный блоб
            format!(
                "Привет!\n\n\
                Я использую защищённый мессенджер EChat.\n\
                Установи приложение по ссылке: {app_url}\n\n\
                После установки ты увидишь моё сообщение автоматически.\n\n\
                ---\n\
                {body_base64}"
            )
        }
    };

    // X-EChat: 1 — наш маркер для быстрой фильтрации
    let extra_headers = vec![("X-EChat".to_string(), "1".to_string())];

    DisguisedEmail {
        subject,
        body,
        extra_headers,
    }
}

/// Проверяет заголовки входящего письма — наше ли это?
///
/// Быстрая проверка: смотрим только на `X-EChat` заголовок.
/// Если заголовка нет — проверяем тело через magic bytes (fallback).
pub fn is_echat_message(headers: &[(&str, &str)], body: Option<&str>) -> bool {
    // Быстрый путь: заголовок
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("x-echat") && value.trim() == "1" {
            return true;
        }
    }

    // Медленный путь: magic bytes в теле (для клиентов которые удаляют заголовки)
    if let Some(body_text) = body {
        // Удаляем переносы строк — base64 может быть разбит на несколько строк
        let body_single_line: String = body_text.chars().filter(|c| !c.is_whitespace()).collect();

        // Проверяем на EncryptedPayload
        if crate::cipher::EncryptedPayload::has_magic_prefix(&body_single_line) {
            return true;
        }

        // Проверяем на HandshakeMessage (пробуем декодировать base64)
        if crate::handshake::HandshakeMessage::from_base64(&body_single_line).is_ok() {
            return true;
        }
    }

    false
}

/// Извлекает base64-блоб из тела письма.
///
/// Для Invite-писем пропускает текстовую часть и находит блоб после "---".
/// Удаляет все переносы строк — base64 может быть разбит на несколько строк.
pub fn extract_payload(body: &str) -> &str {
    // Ищем разделитель приглашения
    if let Some(pos) = body.find("\n---\n") {
        return body[pos + 5..].trim();
    }
    // Иначе всё тело — это payload, но нужно удалить переносы строк
    // Возвращаем оригинальную строку — caller должен сам удалить whitespace
    body.trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_random_subject_not_empty() {
        for _ in 0..20 {
            let s = random_subject();
            assert!(!s.is_empty());
        }
    }

    #[test]
    fn test_build_encrypted_email() {
        let email = build_email("dGVzdA==", BodyKind::EncryptedMessage);
        assert!(!email.subject.is_empty());
        assert_eq!(email.body, "dGVzdA==");
        assert!(
            email
                .extra_headers
                .iter()
                .any(|(k, v)| k == "X-EChat" && v == "1")
        );
    }

    #[test]
    fn test_build_invite_email() {
        let email = build_email(
            "dGVzdA==",
            BodyKind::Invite {
                app_url: "https://echat.app".to_string(),
            },
        );
        assert!(email.body.contains("https://echat.app"));
        assert!(email.body.contains("dGVzdA=="));
    }

    #[test]
    fn test_is_echat_message_by_header() {
        let headers = vec![("X-EChat", "1"), ("From", "alice@mail.ru")];
        assert!(is_echat_message(&headers, None));
    }

    #[test]
    fn test_is_not_echat_message() {
        let headers = vec![("From", "alice@mail.ru"), ("Subject", "Hello")];
        assert!(!is_echat_message(&headers, None));
    }

    #[test]
    fn test_extract_payload_from_invite() {
        let body = "Привет!\n\nУстанови приложение.\n\n---\naGVsbG8=";
        assert_eq!(extract_payload(body), "aGVsbG8=");
    }

    #[test]
    fn test_extract_payload_direct() {
        let body = "  aGVsbG8=  ";
        assert_eq!(extract_payload(body), "aGVsbG8=");
    }
}
