//! StorageAdapter — реализует `core::ports::StoragePort` поверх `crates/storage`.
//!
//! Главная задача: конвертировать типы между слоями:
//!   `storage::models::*`  ↔  `core::models::*`

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use core::{
    models::{
        account::{Account, Provider as CoreProvider},
        contact::{Contact, ContactPublicKeys, ContactStatus as CoreContactStatus},
        conversation::{Conversation, ConversationKind, GroupMember, GroupRole as CoreGroupRole},
        message::{Message, MessageKind as CoreMsgKind, MessageStatus as CoreMsgStatus},
    },
    ports::storage::{
        CreateAccount, CreateContact, CreateMessage, DynStorage, ImapUidEntry,
        StoragePort, UpdateContact,
    },
    Error as CoreError, Result as CoreResult,
};
use storage::{
    models::{
        ContactStatus, GroupRole, MessageKind, MessageStatus, NewAccount, NewContact,
        NewDirectConversation, NewGroupConversation, NewGroupMember, NewMessage,
        Provider, parse_dt, now_iso,
    },
    Database,
};

pub struct StorageAdapter {
    db: Database,
}

impl StorageAdapter {
    pub fn new(db: Database) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { db })
    }
}

// ── Конвертации типов ─────────────────────────────────────────────────────────

fn to_core_provider(p: &Provider) -> CoreProvider {
    match p {
        Provider::MailRu => CoreProvider::MailRu,
        Provider::Yandex => CoreProvider::Yandex,
    }
}

fn from_core_provider(p: &CoreProvider) -> Provider {
    match p {
        CoreProvider::MailRu => Provider::MailRu,
        CoreProvider::Yandex => Provider::Yandex,
    }
}

fn to_core_account(row: storage::models::AccountRow) -> CoreResult<Account> {
    let provider = Provider::from_str(&row.provider)
        .map(|p| to_core_provider(&p))
        .ok_or_else(|| CoreError::Internal(format!("Неизвестный провайдер: {}", row.provider)))?;

    Ok(Account {
        id: Uuid::parse_str(&row.id).map_err(|e| CoreError::Internal(e.to_string()))?,
        email: row.email,
        provider,
        imap_host: row.imap_host,
        imap_port: row.imap_port as u16,
        smtp_host: row.smtp_host,
        smtp_port: row.smtp_port as u16,
        echat_folder: row.echat_folder,
        last_imap_uid: row.last_imap_uid.map(|u| u as u32),
        last_sync_at: row.last_sync_at.as_deref().and_then(parse_dt),
        is_active: row.is_active != 0,
        created_at: parse_dt(&row.created_at).unwrap_or_else(Utc::now),
    })
}

fn to_core_contact(row: storage::models::ContactRow) -> CoreResult<Contact> {
    let status = match row.status.as_str() {
        "pending"      => CoreContactStatus::Pending,
        "active"       => CoreContactStatus::Active,
        _              => CoreContactStatus::Unregistered,
    };

    let public_keys = row.public_keys_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<ContactPublicKeys>(s).ok());

    Ok(Contact {
        id: Uuid::parse_str(&row.id).map_err(|e| CoreError::Internal(e.to_string()))?,
        account_id: Uuid::parse_str(&row.account_id).map_err(|e| CoreError::Internal(e.to_string()))?,
        name: row.name,
        email: row.email,
        avatar: row.avatar,
        status,
        public_keys,
        handshake_at: row.handshake_at.as_deref().and_then(parse_dt),
        created_at: parse_dt(&row.created_at).unwrap_or_else(Utc::now),
        updated_at: parse_dt(&row.updated_at).unwrap_or_else(Utc::now),
    })
}

fn to_core_conversation(row: storage::models::ConversationRow) -> CoreResult<Conversation> {
    let kind = match row.kind.as_str() {
        "direct" => {
            let contact_id = row.contact_id
                .as_deref()
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or_else(|| CoreError::Internal("direct conv без contact_id".into()))?;
            ConversationKind::Direct { contact_id }
        }
        "group" => ConversationKind::Group {
            name: row.group_name.unwrap_or_default(),
            avatar: row.group_avatar,
            members: vec![], // загружается отдельно
        },
        k => return Err(CoreError::Internal(format!("Неизвестный kind: {}", k))),
    };

    Ok(Conversation {
        id: Uuid::parse_str(&row.id).map_err(|e| CoreError::Internal(e.to_string()))?,
        account_id: Uuid::parse_str(&row.account_id).map_err(|e| CoreError::Internal(e.to_string()))?,
        kind,
        last_msg_at: row.last_msg_at.as_deref().and_then(parse_dt),
        last_msg_preview: row.last_msg_preview,
        unread_count: row.unread_count as u32,
        created_at: parse_dt(&row.created_at).unwrap_or_else(Utc::now),
        updated_at: parse_dt(&row.updated_at).unwrap_or_else(Utc::now),
    })
}

fn to_core_group_member(row: storage::models::GroupMemberRow) -> CoreResult<GroupMember> {
    let role = match row.role.as_str() {
        "owner" => CoreGroupRole::Owner,
        "admin" => CoreGroupRole::Admin,
        _       => CoreGroupRole::Member,
    };
    Ok(GroupMember {
        contact_id: Uuid::parse_str(&row.contact_id).map_err(|e| CoreError::Internal(e.to_string()))?,
        role,
        joined_at: parse_dt(&row.joined_at).unwrap_or_else(Utc::now),
        public_key_snapshot: row.public_key_snapshot,
    })
}

fn to_core_message(row: storage::models::MessageRow) -> CoreResult<Message> {
    let kind = match row.kind.as_str() {
        "handshake"   => CoreMsgKind::Handshake,
        "group_event" => CoreMsgKind::GroupEvent,
        _             => CoreMsgKind::Text,
    };
    let status = match row.status.as_str() {
        "queued"    => CoreMsgStatus::Queued,
        "sending"   => CoreMsgStatus::Sending,
        "delivered" => CoreMsgStatus::Delivered,
        "read"      => CoreMsgStatus::Read,
        _           => CoreMsgStatus::Sent,
    };
    Ok(Message {
        id: Uuid::parse_str(&row.id).map_err(|e| CoreError::Internal(e.to_string()))?,
        conversation_id: Uuid::parse_str(&row.conversation_id).map_err(|e| CoreError::Internal(e.to_string()))?,
        account_id: Uuid::parse_str(&row.account_id).map_err(|e| CoreError::Internal(e.to_string()))?,
        from_email: row.from_email,
        body: row.body,
        kind,
        status,
        reply_to: row.reply_to.as_deref().and_then(|s| Uuid::parse_str(s).ok()),
        imap_uid: row.imap_uid.map(|u| u as u32),
        imap_folder: row.imap_folder,
        sent_at: parse_dt(&row.sent_at).unwrap_or_else(Utc::now),
        received_at: row.received_at.as_deref().and_then(parse_dt),
    })
}

fn map_storage_err(e: storage::Error) -> CoreError {
    match e {
        storage::Error::NotFound(s)    => CoreError::NotFound(s),
        storage::Error::Conflict(s)    => CoreError::AlreadyExists(s),
        e                              => CoreError::Storage(e.to_string()),
    }
}

// ── Реализация StoragePort ────────────────────────────────────────────────────

#[async_trait]
impl StoragePort for StorageAdapter {
    // ── Аккаунты ─────────────────────────────────────────────────────────────

    async fn create_account(&self, data: CreateAccount) -> CoreResult<()> {
        self.db.accounts().create(&NewAccount {
            id: data.id,
            email: data.email,
            provider: from_core_provider(&data.provider),
            imap_host: data.imap_host,
            imap_port: data.imap_port,
            smtp_host: data.smtp_host,
            smtp_port: data.smtp_port,
            echat_folder: data.echat_folder,
        }).await.map_err(map_storage_err)
    }

    async fn get_account(&self, id: Uuid) -> CoreResult<Account> {
        let row = self.db.accounts().get_by_id(id).await.map_err(map_storage_err)?;
        to_core_account(row)
    }

    async fn get_account_by_email(&self, email: &str) -> CoreResult<Account> {
        let row = self.db.accounts().get_by_email(email).await.map_err(map_storage_err)?;
        to_core_account(row)
    }

    async fn list_accounts(&self) -> CoreResult<Vec<Account>> {
        let rows = self.db.accounts().list().await.map_err(map_storage_err)?;
        rows.into_iter().map(to_core_account).collect()
    }

    async fn update_account_sync_state(&self, id: Uuid, last_uid: u32) -> CoreResult<()> {
        self.db.accounts().update_sync_state(id, last_uid).await.map_err(map_storage_err)
    }

    async fn delete_account(&self, id: Uuid) -> CoreResult<()> {
        self.db.accounts().delete(id).await.map_err(map_storage_err)
    }

    // ── Контакты ─────────────────────────────────────────────────────────────

    async fn create_contact(&self, data: CreateContact) -> CoreResult<()> {
        self.db.contacts().create(&NewContact {
            id: data.id,
            account_id: data.account_id,
            name: data.name,
            email: data.email,
            avatar: data.avatar,
        }).await.map_err(map_storage_err)
    }

    async fn get_contact(&self, id: Uuid) -> CoreResult<Contact> {
        let row = self.db.contacts().get_by_id(id).await.map_err(map_storage_err)?;
        to_core_contact(row)
    }

    async fn get_contact_by_email(&self, account_id: Uuid, email: &str) -> CoreResult<Contact> {
        let row = self.db.contacts().get_by_email(account_id, email).await.map_err(map_storage_err)?;
        to_core_contact(row)
    }

    async fn list_contacts(&self, account_id: Uuid) -> CoreResult<Vec<Contact>> {
        let rows = self.db.contacts().list(account_id).await.map_err(map_storage_err)?;
        rows.into_iter().map(to_core_contact).collect()
    }

    async fn update_contact(&self, id: Uuid, data: UpdateContact) -> CoreResult<()> {
        self.db.contacts().update(id, &storage::models::UpdateContact {
            name: data.name,
            avatar: data.avatar,
            ..Default::default()
        }).await.map_err(map_storage_err)
    }

    async fn set_contact_pending(&self, id: Uuid) -> CoreResult<()> {
        self.db.contacts().set_pending(id).await.map_err(map_storage_err)
    }

    async fn complete_contact_handshake(&self, id: Uuid, public_keys_json: String) -> CoreResult<()> {
        self.db.contacts().complete_handshake(id, &public_keys_json).await.map_err(map_storage_err)
    }

    async fn delete_contact(&self, id: Uuid) -> CoreResult<()> {
        self.db.contacts().delete(id).await.map_err(map_storage_err)
    }

    // ── Беседы ───────────────────────────────────────────────────────────────

    async fn create_direct_conversation(&self, id: Uuid, account_id: Uuid, contact_id: Uuid) -> CoreResult<()> {
        self.db.conversations().create_direct(&NewDirectConversation { id, account_id, contact_id })
            .await.map_err(map_storage_err)
    }

    async fn create_group_conversation(
        &self,
        id: Uuid,
        account_id: Uuid,
        name: String,
        avatar: Option<Vec<u8>>,
        members: Vec<(Uuid, CoreGroupRole, Option<String>)>,
    ) -> CoreResult<()> {
        let storage_members = members.into_iter().map(|(contact_id, role, snapshot)| {
            NewGroupMember {
                contact_id,
                role: match role {
                    CoreGroupRole::Owner  => GroupRole::Owner,
                    CoreGroupRole::Admin  => GroupRole::Admin,
                    CoreGroupRole::Member => GroupRole::Member,
                },
                public_key_snapshot: snapshot,
            }
        }).collect();

        self.db.conversations().create_group(&NewGroupConversation {
            id, account_id, name, avatar, members: storage_members,
        }).await.map_err(map_storage_err)
    }

    async fn get_conversation(&self, id: Uuid) -> CoreResult<Conversation> {
        let row = self.db.conversations().get_by_id(id).await.map_err(map_storage_err)?;
        to_core_conversation(row)
    }

    async fn find_direct_conversation(&self, account_id: Uuid, contact_id: Uuid) -> CoreResult<Option<Conversation>> {
        match self.db.conversations().find_direct(account_id, contact_id).await.map_err(map_storage_err)? {
            Some(row) => Ok(Some(to_core_conversation(row)?)),
            None => Ok(None),
        }
    }

    async fn list_conversations(&self, account_id: Uuid) -> CoreResult<Vec<Conversation>> {
        let rows = self.db.conversations().list(account_id).await.map_err(map_storage_err)?;
        rows.into_iter().map(to_core_conversation).collect()
    }

    async fn get_group_members(&self, conv_id: Uuid) -> CoreResult<Vec<GroupMember>> {
        let rows = self.db.conversations().get_members(conv_id).await.map_err(map_storage_err)?;
        rows.into_iter().map(to_core_group_member).collect()
    }

    async fn update_conversation_last_message(
        &self, conv_id: Uuid, preview: String,
        sent_at: DateTime<Utc>, increment_unread: bool,
    ) -> CoreResult<()> {
        self.db.conversations()
            .update_last_message(conv_id, &preview, &sent_at.to_rfc3339(), increment_unread)
            .await.map_err(map_storage_err)
    }

    async fn mark_conversation_read(&self, conv_id: Uuid) -> CoreResult<()> {
        self.db.conversations().mark_as_read(conv_id).await.map_err(map_storage_err)
    }

    async fn add_group_member(&self, conv_id: Uuid, contact_id: Uuid, role: CoreGroupRole, pubkey_snapshot: Option<String>) -> CoreResult<()> {
        self.db.conversations().add_member(conv_id, &NewGroupMember {
            contact_id,
            role: match role {
                CoreGroupRole::Owner  => GroupRole::Owner,
                CoreGroupRole::Admin  => GroupRole::Admin,
                CoreGroupRole::Member => GroupRole::Member,
            },
            public_key_snapshot: pubkey_snapshot,
        }).await.map_err(map_storage_err)
    }

    async fn remove_group_member(&self, conv_id: Uuid, contact_id: Uuid) -> CoreResult<()> {
        self.db.conversations().remove_member(conv_id, contact_id).await.map_err(map_storage_err)
    }

    async fn delete_conversation(&self, id: Uuid) -> CoreResult<()> {
        self.db.conversations().delete(id).await.map_err(map_storage_err)
    }

    // ── Сообщения ────────────────────────────────────────────────────────────

    async fn create_message(&self, data: CreateMessage) -> CoreResult<()> {
        self.db.messages().create(&NewMessage {
            id: data.id,
            conversation_id: data.conversation_id,
            account_id: data.account_id,
            from_email: data.from_email,
            body: data.body,
            kind: match data.kind {
                CoreMsgKind::Text       => MessageKind::Text,
                CoreMsgKind::Handshake  => MessageKind::Handshake,
                CoreMsgKind::GroupEvent => MessageKind::GroupEvent,
            },
            status: match data.status {
                CoreMsgStatus::Queued    => MessageStatus::Queued,
                CoreMsgStatus::Sending   => MessageStatus::Sending,
                CoreMsgStatus::Sent      => MessageStatus::Sent,
                CoreMsgStatus::Delivered => MessageStatus::Delivered,
                CoreMsgStatus::Read      => MessageStatus::Read,
            },
            reply_to: data.reply_to,
            imap_uid: data.imap_uid,
            imap_folder: data.imap_folder,
            sent_at: data.sent_at,
        }).await.map_err(map_storage_err)
    }

    async fn message_exists(&self, id: Uuid) -> CoreResult<bool> {
        self.db.messages().exists(id).await.map_err(map_storage_err)
    }

    async fn get_message_history(&self, conv_id: Uuid, before: Option<DateTime<Utc>>, limit: usize) -> CoreResult<Vec<Message>> {
        let before_str = before.map(|dt| dt.to_rfc3339());
        let rows = self.db.messages()
            .get_history(conv_id, before_str.as_deref(), limit as i64)
            .await.map_err(map_storage_err)?;
        rows.into_iter().map(to_core_message).collect()
    }

    async fn update_message_status(&self, id: Uuid, status: CoreMsgStatus) -> CoreResult<()> {
        let s = match status {
            CoreMsgStatus::Queued    => storage::models::MessageStatus::Queued,
            CoreMsgStatus::Sending   => storage::models::MessageStatus::Sending,
            CoreMsgStatus::Sent      => storage::models::MessageStatus::Sent,
            CoreMsgStatus::Delivered => storage::models::MessageStatus::Delivered,
            CoreMsgStatus::Read      => storage::models::MessageStatus::Read,
        };
        self.db.messages().update_status(id, s).await.map_err(map_storage_err)
    }

    async fn get_imap_uids_for_deletion(&self, conv_id: Uuid) -> CoreResult<Vec<ImapUidEntry>> {
        let rows = self.db.messages().get_imap_uids_for_deletion(conv_id).await.map_err(map_storage_err)?;
        Ok(rows.into_iter().filter_map(|r| {
            Some(ImapUidEntry {
                uid: r.imap_uid? as u32,
                folder: r.imap_folder?,
            })
        }).collect())
    }

    async fn delete_conversation_messages(&self, conv_id: Uuid) -> CoreResult<()> {
        self.db.messages().delete_conversation_messages(conv_id).await.map_err(map_storage_err)?;
        Ok(())
    }

    async fn get_queued_messages(&self, account_id: Uuid) -> CoreResult<Vec<Message>> {
        let rows = self.db.messages().get_queued(account_id).await.map_err(map_storage_err)?;
        rows.into_iter().map(to_core_message).collect()
    }
}
