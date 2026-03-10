//! Интеграционные тесты email крейта.
//!
//! Тесты разделены на два уровня:
//!
//! 1. **Unit-тесты** (без сети) — codec, providers, типы
//! 2. **Live-тесты** (требуют реальный аккаунт) — помечены `#[ignore]`,
//!    запускаются вручную: `cargo test -- --ignored`

use email::{
    codec::{self, ChatEnvelope, EnvelopeKind},
    providers::ProviderConfig,
    types::{IncomingMessage, MessageUid, RawEmailHeaders},
};
use uuid::Uuid;

// ── Unit тесты (без сети) ─────────────────────────────────────────────────────

#[test]
fn test_provider_detect_mailru() {
    let cfg = ProviderConfig::detect("user@mail.ru", "pass").unwrap();
    assert!(matches!(
        cfg.provider,
        email::providers::Provider::MailRu
    ));
    assert_eq!(cfg.imap.host, "imap.mail.ru");
    assert_eq!(cfg.imap.port, 993);
    assert_eq!(cfg.smtp.host, "smtp.mail.ru");
    assert_eq!(cfg.smtp.port, 465);
}

#[test]
fn test_provider_detect_yandex_domains() {
    for domain in &["yandex.ru", "ya.ru", "yandex.com", "yandex.kz"] {
        let email = format!("user@{}", domain);
        let cfg = ProviderConfig::detect(&email, "pass").unwrap();
        assert!(matches!(
            cfg.provider,
            email::providers::Provider::Yandex
        ));
        assert_eq!(cfg.imap.host, "imap.yandex.ru");
        assert_eq!(cfg.smtp.host, "smtp.yandex.ru");
    }
}

#[test]
fn test_provider_detect_mailru_aliases() {
    for domain in &["inbox.ru", "list.ru", "bk.ru"] {
        let email = format!("user@{}", domain);
        let cfg = ProviderConfig::detect(&email, "pass").unwrap();
        assert!(
            matches!(cfg.provider, email::providers::Provider::MailRu),
            "Домен {} должен определяться как Mail.ru",
            domain
        );
    }
}

#[test]
fn test_provider_detect_unknown() {
    assert!(ProviderConfig::detect("user@gmail.com", "pass").is_none());
    assert!(ProviderConfig::detect("user@example.com", "pass").is_none());
}

#[test]
fn test_codec_full_cycle() {
    use encryption::{
        cipher,
        keypair::IdentityKeypair,
        session::derive_from_bytes,
    };

    // Ключи
    let alice = IdentityKeypair::generate();
    let bob = IdentityKeypair::generate();

    let alice_shared = derive_from_bytes(
        alice.secret_key(),
        &bob.public_keys().x25519,
        "direct-chat",
    )
    .unwrap();
    let bob_shared = derive_from_bytes(
        bob.secret_key(),
        &alice.public_keys().x25519,
        "direct-chat",
    )
    .unwrap();

    // Alice создаёт и шифрует envelope
    let conv_id = Uuid::new_v4();
    let envelope = ChatEnvelope::new_message(conv_id, "Привет, Боб!".to_string(), None);
    let envelope_bytes = envelope.to_bytes().unwrap();

    let payload = cipher::encrypt(&envelope_bytes, &alice_shared).unwrap();
    let payload_b64 = payload.to_base64();

    // Формируем OutgoingMessage
    let outgoing = codec::encode_message(
        "alice@mail.ru",
        &["bob@yandex.ru".to_string()],
        &payload_b64,
    );

    // Симулируем получение: IncomingMessage
    let incoming = IncomingMessage {
        uid: MessageUid(42),
        folder: "EChat".to_string(),
        from: "alice@mail.ru".to_string(),
        to: vec!["bob@yandex.ru".to_string()],
        subject: outgoing.subject.clone(),
        body: outgoing.body.clone(),
        headers: RawEmailHeaders(
            outgoing
                .extra_headers
                .iter()
                .cloned()
                .collect(),
        ),
        date: chrono::Utc::now(),
    };

    // Проверяем что это echat-письмо
    assert!(codec::is_echat_message(&incoming));

    // Извлекаем и расшифровываем
    let extracted = codec::extract_payload(&incoming);
    let received_payload = encryption::cipher::EncryptedPayload::from_base64(extracted).unwrap();
    let decrypted_bytes = cipher::decrypt(&received_payload, &bob_shared).unwrap();

    let decoded_envelope = ChatEnvelope::from_bytes(&decrypted_bytes).unwrap();
    assert_eq!(decoded_envelope.msg_id, envelope.msg_id);
    assert_eq!(decoded_envelope.body, Some("Привет, Боб!".to_string()));
    assert_eq!(decoded_envelope.kind, EnvelopeKind::Message);
}

#[test]
fn test_non_echat_message_rejected() {
    let msg = IncomingMessage {
        uid: MessageUid(1),
        folder: "INBOX".to_string(),
        from: "someone@mail.ru".to_string(),
        to: vec!["me@yandex.ru".to_string()],
        subject: "Привет!".to_string(),
        body: "Как дела?".to_string(),
        headers: RawEmailHeaders::default(),
        date: chrono::Utc::now(),
    };
    assert!(!codec::is_echat_message(&msg));
}

// ── Live тесты (требуют реальный аккаунт) ─────────────────────────────────────
//
// Чтобы запустить:
// ECHAT_TEST_EMAIL=user@mail.ru ECHAT_TEST_PASS=app_password \
//   cargo test -p email -- --ignored --nocapture

#[tokio::test]
#[ignore = "требует реальный почтовый аккаунт"]
async fn live_test_imap_connect_mailru() {
    let email = std::env::var("ECHAT_TEST_EMAIL")
        .expect("Укажи ECHAT_TEST_EMAIL");
    let pass = std::env::var("ECHAT_TEST_PASS")
        .expect("Укажи ECHAT_TEST_PASS");

    let config = ProviderConfig::detect(&email, &pass)
        .expect("Неизвестный провайдер");

    let client = email::EmailClient::connect(config)
        .await
        .expect("Подключение не удалось");

    client
        .ensure_echat_folder()
        .await
        .expect("Не удалось создать папку EChat");

    let messages = client
        .fetch_new_messages(None)
        .await
        .expect("Не удалось получить письма");

    println!("Писем в EChat: {}", messages.len());
    for msg in &messages {
        println!(
            "  UID={} from={} subject='{}'",
            msg.uid, msg.from, msg.subject
        );
    }
}

#[tokio::test]
#[ignore = "требует два реальных аккаунта и отправляет письмо"]
async fn live_test_send_and_receive() {
    let from_email = std::env::var("ECHAT_FROM_EMAIL").unwrap();
    let from_pass = std::env::var("ECHAT_FROM_PASS").unwrap();
    let to_email = std::env::var("ECHAT_TO_EMAIL").unwrap();

    let config = ProviderConfig::detect(&from_email, &from_pass).unwrap();
    let client = email::EmailClient::connect(config).await.unwrap();

    // Отправляем тестовое зашифрованное письмо
    let fake_payload = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        b"\xEC\xC4\xa7\x01test_payload",
    );
    let msg = codec::encode_message(&from_email, &[to_email], &fake_payload);

    client.send_message(msg).await.unwrap();
    println!("Письмо отправлено!");
}
