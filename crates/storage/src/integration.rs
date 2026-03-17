//! Тесты storage крейта.
//!
//! Все тесты используют in-memory SQLite — быстро, без файлов, изолированно.
//! Каждый тест получает свежую БД через `test_db()`.

use storage::{
    models::{
        ContactStatus, GroupRole, MessageKind, MessageStatus,
        NewAccount, NewContact, NewDirectConversation, NewGroupConversation,
        NewGroupMember, NewMessage, Provider,
    },
    Database,
};
use uuid::Uuid;

async fn test_db() -> Database {
    Database::open_in_memory().await.expect("in-memory DB")
}

fn new_account_id() -> Uuid {
    Uuid::new_v4()
}

async fn create_test_account(db: &Database) -> Uuid {
    let id = Uuid::new_v4();
    db.accounts()
        .create(&NewAccount {
            id,
            email: format!("user{}@mail.ru", &id.to_string()[..8]),
            provider: Provider::MailRu,
            imap_host: "imap.mail.ru".into(),
            imap_port: 993,
            smtp_host: "smtp.mail.ru".into(),
            smtp_port: 465,
            echat_folder: "EChat".into(),
        })
        .await
        .expect("create account");
    id
}

async fn create_test_contact(db: &Database, account_id: Uuid, email: &str) -> Uuid {
    let id = Uuid::new_v4();
    db.contacts()
        .create(&NewContact {
            id,
            account_id,
            name: "Test User".into(),
            email: email.into(),
            avatar: None,
        })
        .await
        .expect("create contact");
    id
}

// ── Account ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_account_create_and_get() {
    let db = test_db().await;
    let acc_id = create_test_account(&db).await;

    let row = db.accounts().get_by_id(acc_id).await.unwrap();
    assert!(row.email.contains("@mail.ru"));
    assert_eq!(row.provider, "mailru");
    assert_eq!(row.is_active, 1);
}

#[tokio::test]
async fn test_account_duplicate_email_fails() {
    let db = test_db().await;
    let email = "duplicate@mail.ru";

    db.accounts()
        .create(&NewAccount {
            id: Uuid::new_v4(),
            email: email.into(),
            provider: Provider::MailRu,
            imap_host: "imap.mail.ru".into(),
            imap_port: 993,
            smtp_host: "smtp.mail.ru".into(),
            smtp_port: 465,
            echat_folder: "EChat".into(),
        })
        .await
        .unwrap();

    let result = db
        .accounts()
        .create(&NewAccount {
            id: Uuid::new_v4(),
            email: email.into(),
            provider: Provider::MailRu,
            imap_host: "imap.mail.ru".into(),
            imap_port: 993,
            smtp_host: "smtp.mail.ru".into(),
            smtp_port: 465,
            echat_folder: "EChat".into(),
        })
        .await;

    assert!(matches!(result, Err(storage::Error::Conflict(_))));
}

#[tokio::test]
async fn test_account_update_sync_state() {
    let db = test_db().await;
    let acc_id = create_test_account(&db).await;

    db.accounts().update_sync_state(acc_id, 42).await.unwrap();

    let row = db.accounts().get_by_id(acc_id).await.unwrap();
    assert_eq!(row.last_imap_uid, Some(42));
    assert!(row.last_sync_at.is_some());
}

#[tokio::test]
async fn test_account_delete() {
    let db = test_db().await;
    let acc_id = create_test_account(&db).await;

    db.accounts().delete(acc_id).await.unwrap();

    let result = db.accounts().get_by_id(acc_id).await;
    assert!(matches!(result, Err(storage::Error::NotFound(_))));
}

// ── Contact ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_contact_create_list() {
    let db = test_db().await;
    let acc_id = create_test_account(&db).await;

    create_test_contact(&db, acc_id, "alice@yandex.ru").await;
    create_test_contact(&db, acc_id, "bob@mail.ru").await;

    let contacts = db.contacts().list(acc_id).await.unwrap();
    assert_eq!(contacts.len(), 2);
    // Должны быть отсортированы по имени
    assert_eq!(contacts[0].name, "Test User");
}

#[tokio::test]
async fn test_contact_handshake_flow() {
    let db = test_db().await;
    let acc_id = create_test_account(&db).await;
    let contact_id = create_test_contact(&db, acc_id, "bob@yandex.ru").await;

    // Начальный статус — nokey
    let c = db.contacts().get_by_id(contact_id).await.unwrap();
    assert_eq!(c.status, "nokey");
    assert!(c.public_keys_json.is_none());

    // Получили ответ — haskey
    let keys_json = r#"{"x25519":"aabbcc","ed25519":"ddeeff"}"#;
    db.contacts()
        .complete_handshake(contact_id, keys_json)
        .await
        .unwrap();

    let c = db.contacts().get_by_id(contact_id).await.unwrap();
    assert_eq!(c.status, "haskey");
    assert_eq!(c.public_keys_json.as_deref(), Some(keys_json));
    assert!(c.handshake_at.is_some());
}

#[tokio::test]
async fn test_contact_isolation_between_accounts() {
    let db = test_db().await;
    let acc1 = create_test_account(&db).await;
    let acc2 = create_test_account(&db).await;

    create_test_contact(&db, acc1, "shared@mail.ru").await;
    create_test_contact(&db, acc2, "shared@mail.ru").await;

    // Каждый аккаунт видит только свои контакты
    let c1 = db.contacts().list(acc1).await.unwrap();
    let c2 = db.contacts().list(acc2).await.unwrap();
    assert_eq!(c1.len(), 1);
    assert_eq!(c2.len(), 1);
    assert_ne!(c1[0].id, c2[0].id);
}

// ── Conversation ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_direct_conversation_create() {
    let db = test_db().await;
    let acc_id = create_test_account(&db).await;
    let contact_id = create_test_contact(&db, acc_id, "bob@yandex.ru").await;

    let conv_id = Uuid::new_v4();
    db.conversations()
        .create_direct(&NewDirectConversation {
            id: conv_id,
            account_id: acc_id,
            contact_id,
        })
        .await
        .unwrap();

    let conv = db.conversations().get_by_id(conv_id).await.unwrap();
    assert_eq!(conv.kind, "direct");
    assert_eq!(conv.contact_id.as_deref(), Some(contact_id.to_string().as_str()));
    assert_eq!(conv.unread_count, 0);
}

#[tokio::test]
async fn test_group_conversation_with_members() {
    let db = test_db().await;
    let acc_id = create_test_account(&db).await;
    let alice_id = create_test_contact(&db, acc_id, "alice@mail.ru").await;
    let bob_id = create_test_contact(&db, acc_id, "bob@yandex.ru").await;
    let carol_id = create_test_contact(&db, acc_id, "carol@mail.ru").await;

    let conv_id = Uuid::new_v4();
    db.conversations()
        .create_group(&NewGroupConversation {
            id: conv_id,
            account_id: acc_id,
            name: "Team Chat".into(),
            avatar: None,
            members: vec![
                NewGroupMember { contact_id: alice_id, role: GroupRole::Owner, public_key_snapshot: None },
                NewGroupMember { contact_id: bob_id,   role: GroupRole::Member, public_key_snapshot: None },
                NewGroupMember { contact_id: carol_id, role: GroupRole::Member, public_key_snapshot: None },
            ],
        })
        .await
        .unwrap();

    let members = db.conversations().get_members(conv_id).await.unwrap();
    assert_eq!(members.len(), 3);

    let owner = members.iter().find(|m| m.contact_id == alice_id.to_string()).unwrap();
    assert_eq!(owner.role, "owner");
}

#[tokio::test]
async fn test_group_add_remove_member() {
    let db = test_db().await;
    let acc_id = create_test_account(&db).await;
    let alice_id = create_test_contact(&db, acc_id, "alice@mail.ru").await;
    let bob_id = create_test_contact(&db, acc_id, "bob@yandex.ru").await;

    let conv_id = Uuid::new_v4();
    db.conversations()
        .create_group(&NewGroupConversation {
            id: conv_id,
            account_id: acc_id,
            name: "Group".into(),
            avatar: None,
            members: vec![
                NewGroupMember { contact_id: alice_id, role: GroupRole::Owner, public_key_snapshot: None },
            ],
        })
        .await
        .unwrap();

    // Добавляем Bob
    db.conversations()
        .add_member(conv_id, &NewGroupMember {
            contact_id: bob_id,
            role: GroupRole::Member,
            public_key_snapshot: None,
        })
        .await
        .unwrap();

    assert_eq!(db.conversations().get_members(conv_id).await.unwrap().len(), 2);

    // Удаляем Bob
    db.conversations().remove_member(conv_id, bob_id).await.unwrap();
    assert_eq!(db.conversations().get_members(conv_id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn test_conversation_unread_counter() {
    let db = test_db().await;
    let acc_id = create_test_account(&db).await;
    let contact_id = create_test_contact(&db, acc_id, "bob@yandex.ru").await;
    let conv_id = Uuid::new_v4();

    db.conversations()
        .create_direct(&NewDirectConversation { id: conv_id, account_id: acc_id, contact_id })
        .await
        .unwrap();

    // Три входящих сообщения
    for _ in 0..3 {
        db.conversations()
            .update_last_message(conv_id, "Привет!", "2026-03-10T10:00:00Z", true)
            .await
            .unwrap();
    }
    let conv = db.conversations().get_by_id(conv_id).await.unwrap();
    assert_eq!(conv.unread_count, 3);

    // Пользователь открыл чат
    db.conversations().mark_as_read(conv_id).await.unwrap();
    let conv = db.conversations().get_by_id(conv_id).await.unwrap();
    assert_eq!(conv.unread_count, 0);
}

// ── Message ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_message_create_and_history() {
    let db = test_db().await;
    let acc_id = create_test_account(&db).await;
    let contact_id = create_test_contact(&db, acc_id, "bob@yandex.ru").await;
    let conv_id = Uuid::new_v4();

    db.conversations()
        .create_direct(&NewDirectConversation { id: conv_id, account_id: acc_id, contact_id })
        .await
        .unwrap();

    // Вставляем 5 сообщений
    for i in 0..5u32 {
        db.messages()
            .create(&NewMessage {
                id: Uuid::new_v4(),
                conversation_id: conv_id,
                account_id: acc_id,
                from_email: "alice@mail.ru".into(),
                body: Some(format!("Сообщение {}", i)),
                kind: MessageKind::Text,
                status: MessageStatus::Sent,
                reply_to: None,
                imap_uid: Some(100 + i),
                imap_folder: Some("EChat".into()),
                sent_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
    }

    let history = db.messages().get_history(conv_id, None, 10).await.unwrap();
    assert_eq!(history.len(), 5);
    // Порядок должен быть ASC (старые первые)
    assert!(history[0].body.as_deref().unwrap().contains("0"));
    assert!(history[4].body.as_deref().unwrap().contains("4"));
}

#[tokio::test]
async fn test_message_imap_uids_for_deletion() {
    let db = test_db().await;
    let acc_id = create_test_account(&db).await;
    let contact_id = create_test_contact(&db, acc_id, "bob@yandex.ru").await;
    let conv_id = Uuid::new_v4();

    db.conversations()
        .create_direct(&NewDirectConversation { id: conv_id, account_id: acc_id, contact_id })
        .await
        .unwrap();

    // 3 сообщения с UID, 1 без (исходящее ещё не доставлено)
    for i in 0..3u32 {
        db.messages()
            .create(&NewMessage {
                id: Uuid::new_v4(),
                conversation_id: conv_id,
                account_id: acc_id,
                from_email: "bob@yandex.ru".into(),
                body: Some("msg".into()),
                kind: MessageKind::Text,
                status: MessageStatus::Delivered,
                reply_to: None,
                imap_uid: Some(200 + i),
                imap_folder: Some("EChat".into()),
                sent_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
    }
    db.messages()
        .create(&NewMessage {
            id: Uuid::new_v4(),
            conversation_id: conv_id,
            account_id: acc_id,
            from_email: "alice@mail.ru".into(),
            body: Some("queued".into()),
            kind: MessageKind::Text,
            status: MessageStatus::Queued,
            reply_to: None,
            imap_uid: None,   // ← нет UID
            imap_folder: None,
            sent_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    let uids = db.messages().get_imap_uids_for_deletion(conv_id).await.unwrap();
    // Только 3 записи с UID
    assert_eq!(uids.len(), 3);
    assert!(uids.iter().all(|r| r.imap_uid.is_some()));
}

#[tokio::test]
async fn test_cascade_delete_conversation() {
    let db = test_db().await;
    let acc_id = create_test_account(&db).await;
    let contact_id = create_test_contact(&db, acc_id, "bob@yandex.ru").await;
    let conv_id = Uuid::new_v4();

    db.conversations()
        .create_direct(&NewDirectConversation { id: conv_id, account_id: acc_id, contact_id })
        .await
        .unwrap();

    db.messages()
        .create(&NewMessage {
            id: Uuid::new_v4(),
            conversation_id: conv_id,
            account_id: acc_id,
            from_email: "bob@yandex.ru".into(),
            body: Some("test".into()),
            kind: MessageKind::Text,
            status: MessageStatus::Delivered,
            reply_to: None,
            imap_uid: Some(1),
            imap_folder: Some("EChat".into()),
            sent_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    // Удаляем беседу — сообщения должны удалиться каскадно
    db.conversations().delete(conv_id).await.unwrap();

    let history = db.messages().get_history(conv_id, None, 100).await.unwrap();
    assert!(history.is_empty());
}
