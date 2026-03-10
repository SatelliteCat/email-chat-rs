//! Интеграционные тесты — полный цикл direct и group чатов.

use encryption::prelude::*;

/// Полный цикл: handshake → шифрование → расшифровка (direct чат)
#[test]
fn test_full_direct_chat_flow() {
    // 1. Alice и Bob генерируют ключи
    let alice_kp = IdentityKeypair::generate();
    let bob_kp = IdentityKeypair::generate();

    // 2. Handshake: Alice отправляет Init
    let alice_init =
        encryption::handshake::HandshakeMessage::new_init(&alice_kp, "alice@mail.ru");
    assert!(alice_init.verify(3600).is_ok());

    // 3. Bob получает Init, проверяет, отвечает Ack
    assert!(alice_init.verify(3600).is_ok());
    let bob_ack = encryption::handshake::HandshakeMessage::new_ack(&bob_kp, "bob@yandex.ru");
    assert!(bob_ack.verify(3600).is_ok());

    // 4. Оба вычисляют shared secret
    let alice_secret = encryption::session::derive_from_bytes(
        alice_kp.secret_key(),
        &bob_kp.public_keys().x25519,
        "direct-chat",
    )
    .unwrap();

    let bob_secret = encryption::session::derive_from_bytes(
        bob_kp.secret_key(),
        &alice_kp.public_keys().x25519,
        "direct-chat",
    )
    .unwrap();

    // 5. Shared secret одинаков
    assert_eq!(alice_secret.as_bytes(), bob_secret.as_bytes());

    // 6. Alice шифрует сообщение
    let message = b"Hello Bob! This is a secret message.";
    let payload = cipher::encrypt(message, &alice_secret).unwrap();

    // 7. Маскируем для email
    let b64 = payload.to_base64();
    let email = disguise::build_email(&b64, disguise::BodyKind::EncryptedMessage);
    assert!(!email.subject.is_empty());

    // 8. Симулируем получение: проверяем заголовки, извлекаем payload
    let headers: Vec<(&str, &str)> = email
        .extra_headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert!(disguise::is_echat_message(&headers, None));

    let extracted = disguise::extract_payload(&email.body);
    let received_payload = EncryptedPayload::from_base64(extracted).unwrap();

    // 9. Bob расшифровывает
    let decrypted = cipher::decrypt(&received_payload, &bob_secret).unwrap();
    assert_eq!(decrypted, message);
}

/// Полный цикл группового чата
#[test]
fn test_full_group_chat_flow() {
    use encryption::group::{GroupCipher, GroupMember};

    let alice = IdentityKeypair::generate();
    let bob = IdentityKeypair::generate();
    let carol = IdentityKeypair::generate();

    let group_id = "550e8400-e29b-41d4-a716-446655440000";
    let cipher = GroupCipher::new(group_id);

    let members = vec![
        GroupMember {
            email: "alice@mail.ru".into(),
            public_key_bytes: *alice.public_key().as_bytes(),
        },
        GroupMember {
            email: "bob@yandex.ru".into(),
            public_key_bytes: *bob.public_key().as_bytes(),
        },
        GroupMember {
            email: "carol@mail.ru".into(),
            public_key_bytes: *carol.public_key().as_bytes(),
        },
    ];

    let message = b"Hello, everyone!";
    let payload = cipher.encrypt(message, &alice, &members).unwrap();

    // Alice читает своё сообщение
    let alice_decrypted = cipher
        .decrypt(&payload, &alice, "alice@mail.ru", alice.public_key().as_bytes())
        .unwrap();
    assert_eq!(alice_decrypted, message);

    // Bob читает
    let bob_decrypted = cipher
        .decrypt(&payload, &bob, "bob@yandex.ru", alice.public_key().as_bytes())
        .unwrap();
    assert_eq!(bob_decrypted, message);

    // Carol читает
    let carol_decrypted = cipher
        .decrypt(&payload, &carol, "carol@mail.ru", alice.public_key().as_bytes())
        .unwrap();
    assert_eq!(carol_decrypted, message);
}

/// Экспорт и восстановление ключа сохраняют ту же переписку
#[test]
fn test_key_export_preserves_chat_history() {
    use encryption::export::{export_keypair, import_keypair, ExportFormat};

    let alice = IdentityKeypair::generate();
    let bob = IdentityKeypair::generate();

    // Alice шифрует сообщение
    let shared = encryption::session::derive_from_bytes(
        alice.secret_key(),
        &bob.public_keys().x25519,
        "direct-chat",
    )
    .unwrap();
    let payload = cipher::encrypt(b"важное сообщение", &shared).unwrap();

    // Alice экспортирует и импортирует ключ
    let exported = export_keypair(&alice, "secret123", ExportFormat::Base64).unwrap();
    let alice_restored = import_keypair(&exported, "secret123").unwrap();

    // Восстановленный ключ даёт тот же shared secret
    let restored_shared = encryption::session::derive_from_bytes(
        alice_restored.secret_key(),
        &bob.public_keys().x25519,
        "direct-chat",
    )
    .unwrap();

    assert_eq!(shared.as_bytes(), restored_shared.as_bytes());

    // Bob по-прежнему может расшифровать
    let bob_shared = encryption::session::derive_from_bytes(
        bob.secret_key(),
        &alice_restored.public_keys().x25519,
        "direct-chat",
    )
    .unwrap();
    let decrypted = cipher::decrypt(&payload, &bob_shared).unwrap();
    assert_eq!(decrypted, b"важное сообщение");
}
