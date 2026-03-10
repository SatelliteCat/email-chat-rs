//! Тесты core крейта.
//!
//! Тестируем то что можно без реальных реализаций портов:
//! EventBus, доменные модели, конфигурацию, in-memory keystore.

use core::{
    events::{ChatEvent, EventBus},
    models::{
        contact::ContactStatus,
        conversation::GroupRole,
        message::MessageStatus,
    },
    ports::keystore::{InMemoryKeystore, KeystorePort, SERVICE_MAIL, SERVICE_IDENTITY,
        app_password_key, identity_seed_key},
    AppConfig,
};
use uuid::Uuid;

// ── EventBus ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_event_bus_emit_and_receive() {
    let bus = EventBus::new(16);
    let mut rx = bus.subscribe();

    let conv_id = Uuid::new_v4();
    bus.emit(ChatEvent::SyncStateChanged { connected: true });

    let event = rx.recv().await.unwrap();
    assert!(matches!(event, ChatEvent::SyncStateChanged { connected: true }));
}

#[tokio::test]
async fn test_event_bus_multiple_subscribers() {
    let bus = EventBus::new(16);
    let mut rx1 = bus.subscribe();
    let mut rx2 = bus.subscribe();

    bus.emit(ChatEvent::SyncStateChanged { connected: false });

    let e1 = rx1.recv().await.unwrap();
    let e2 = rx2.recv().await.unwrap();
    assert!(matches!(e1, ChatEvent::SyncStateChanged { connected: false }));
    assert!(matches!(e2, ChatEvent::SyncStateChanged { connected: false }));
}

#[tokio::test]
async fn test_event_bus_no_subscribers_no_panic() {
    let bus = EventBus::new(16);
    // Нет подписчиков — не должно паниковать
    bus.emit(ChatEvent::SyncStateChanged { connected: true });
}

// ── InMemoryKeystore ─────────────────────────────────────────────────────────

#[tokio::test]
async fn test_keystore_set_get() {
    let ks = InMemoryKeystore::new();

    ks.set(SERVICE_MAIL, "key1", b"secret_value").await.unwrap();
    let val = ks.get(SERVICE_MAIL, "key1").await.unwrap();
    assert_eq!(val, Some(b"secret_value".to_vec()));
}

#[tokio::test]
async fn test_keystore_missing_key_returns_none() {
    let ks = InMemoryKeystore::new();
    let val = ks.get(SERVICE_MAIL, "nonexistent").await.unwrap();
    assert_eq!(val, None);
}

#[tokio::test]
async fn test_keystore_delete() {
    let ks = InMemoryKeystore::new();
    ks.set(SERVICE_MAIL, "key", b"val").await.unwrap();
    ks.delete(SERVICE_MAIL, "key").await.unwrap();
    let val = ks.get(SERVICE_MAIL, "key").await.unwrap();
    assert_eq!(val, None);
}

#[tokio::test]
async fn test_keystore_different_services_isolated() {
    let ks = InMemoryKeystore::new();
    ks.set(SERVICE_MAIL, "key", b"mail_secret").await.unwrap();
    ks.set(SERVICE_IDENTITY, "key", b"identity_secret").await.unwrap();

    let mail = ks.get(SERVICE_MAIL, "key").await.unwrap().unwrap();
    let identity = ks.get(SERVICE_IDENTITY, "key").await.unwrap().unwrap();

    assert_eq!(mail, b"mail_secret");
    assert_eq!(identity, b"identity_secret");
}

#[tokio::test]
async fn test_keystore_overwrite() {
    let ks = InMemoryKeystore::new();
    ks.set(SERVICE_MAIL, "key", b"old").await.unwrap();
    ks.set(SERVICE_MAIL, "key", b"new").await.unwrap();
    let val = ks.get(SERVICE_MAIL, "key").await.unwrap().unwrap();
    assert_eq!(val, b"new");
}

// ── Доменные модели ───────────────────────────────────────────────────────────

#[test]
fn test_group_role_permissions() {
    assert!(GroupRole::Owner.can_add_members());
    assert!(GroupRole::Admin.can_add_members());
    assert!(!GroupRole::Member.can_add_members());

    assert!(GroupRole::Owner.can_remove_members());
    assert!(GroupRole::Admin.can_remove_members());
    assert!(!GroupRole::Member.can_remove_members());
}

#[test]
fn test_message_is_incoming() {
    use core::models::message::Message;
    use chrono::Utc;

    let msg = Message {
        id: Uuid::new_v4(),
        conversation_id: Uuid::new_v4(),
        account_id: Uuid::new_v4(),
        from_email: "bob@yandex.ru".to_string(),
        body: Some("Hi".to_string()),
        kind: core::models::message::MessageKind::Text,
        status: MessageStatus::Delivered,
        reply_to: None,
        imap_uid: Some(1),
        imap_folder: Some("EChat".to_string()),
        sent_at: Utc::now(),
        received_at: Some(Utc::now()),
    };

    assert!(msg.is_incoming("alice@mail.ru"));
    assert!(!msg.is_incoming("bob@yandex.ru"));
    // Регистронезависимо
    assert!(!msg.is_incoming("BOB@YANDEX.RU"));
}

// ── Ключевые имена keystore ──────────────────────────────────────────────────

#[test]
fn test_keystore_key_names() {
    let email = "alice@mail.ru";
    let account_id = "550e8400-e29b-41d4-a716-446655440000";

    assert_eq!(app_password_key(email), "app_password:alice@mail.ru");
    assert_eq!(
        identity_seed_key(account_id),
        "identity_seed:550e8400-e29b-41d4-a716-446655440000"
    );
}

// ── AppConfig ─────────────────────────────────────────────────────────────────

#[test]
fn test_app_config_defaults() {
    let config = AppConfig::default();
    assert_eq!(config.app_download_url, "https://echat.app");
    assert!(config.event_bus_capacity > 0);
}
