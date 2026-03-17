//! Трейт StoragePort — абстракция над персистентным хранилищем.
//!
//! Фасад над всеми репозиториями `crates/storage`.
//! При тестировании подставляется in-memory реализация.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    Result,
    models::{
        account::Account,
        contact::Contact,
        conversation::{Conversation, GroupMember, GroupRole},
        message::{Message, MessageKind, MessageStatus},
    },
};

// ── Структуры для создания/обновления ────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CreateAccount {
    pub id: Uuid,
    pub email: String,
    pub provider: crate::models::account::Provider,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub echat_folder: String,
}

#[derive(Debug, Clone)]
pub struct CreateContact {
    pub id: Uuid,
    pub account_id: Uuid,
    pub name: String,
    pub email: String,
    pub avatar: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateContact {
    pub name: Option<String>,
    pub avatar: Option<Option<Vec<u8>>>,
}

#[derive(Debug, Clone)]
pub struct CreateMessage {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub account_id: Uuid,
    pub from_email: String,
    pub body: Option<String>,
    pub kind: MessageKind,
    pub status: MessageStatus,
    pub reply_to: Option<Uuid>,
    pub imap_uid: Option<u32>,
    pub imap_folder: Option<String>,
    pub sent_at: DateTime<Utc>,
    pub error_message: Option<String>,
}

impl CreateMessage {
    pub fn reply_to_account_id(&self) -> Option<Uuid> {
        // reply_to всегда принадлежит тому же аккаунту что и сообщение
        self.reply_to.map(|_| self.account_id)
    }
}

/// UID на IMAP сервере — для удаления писем.
#[derive(Debug, Clone)]
pub struct ImapUidEntry {
    pub uid: u32,
    pub folder: String,
}

/// Ключи диалога.
#[derive(Debug, Clone)]
pub struct ConversationKeys {
    pub conversation_id: Uuid,
    pub my_keypair_json: Option<String>,
    pub their_public_key_json: Option<String>,
    pub is_active: bool,
}

// ── Трейт ─────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait StoragePort: Send + Sync + 'static {
    // ── Аккаунты ─────────────────────────────────────────────────────────────

    async fn create_account(&self, data: CreateAccount) -> Result<()>;
    async fn get_account(&self, id: Uuid) -> Result<Account>;
    async fn get_account_by_email(&self, email: &str) -> Result<Account>;
    async fn list_accounts(&self) -> Result<Vec<Account>>;
    async fn update_account_sync_state(&self, id: Uuid, last_uid: u32) -> Result<()>;
    async fn delete_account(&self, id: Uuid) -> Result<()>;

    // ── Контакты ─────────────────────────────────────────────────────────────

    async fn create_contact(&self, data: CreateContact) -> Result<()>;
    async fn get_contact(&self, id: Uuid) -> Result<Contact>;
    async fn get_contact_by_email(&self, account_id: Uuid, email: &str) -> Result<Contact>;
    async fn list_contacts(&self, account_id: Uuid) -> Result<Vec<Contact>>;
    async fn update_contact(&self, id: Uuid, data: UpdateContact) -> Result<()>;
    async fn complete_contact_handshake(&self, id: Uuid, public_keys_json: String) -> Result<()>;
    async fn delete_contact(&self, id: Uuid) -> Result<()>;

    // ── Беседы ───────────────────────────────────────────────────────────────

    async fn create_direct_conversation(
        &self,
        id: Uuid,
        account_id: Uuid,
        contact_id: Uuid,
    ) -> Result<()>;

    async fn create_group_conversation(
        &self,
        id: Uuid,
        account_id: Uuid,
        name: String,
        avatar: Option<Vec<u8>>,
        members: Vec<(Uuid, GroupRole, Option<String>)>, // (contact_id, role, pubkey_snapshot)
    ) -> Result<()>;

    async fn get_conversation(&self, id: Uuid) -> Result<Conversation>;
    async fn find_direct_conversation(
        &self,
        account_id: Uuid,
        contact_id: Uuid,
    ) -> Result<Option<Conversation>>;
    async fn list_conversations(&self, account_id: Uuid) -> Result<Vec<Conversation>>;
    async fn get_group_members(&self, conv_id: Uuid) -> Result<Vec<GroupMember>>;

    async fn update_conversation_last_message(
        &self,
        conv_id: Uuid,
        preview: String,
        sent_at: DateTime<Utc>,
        increment_unread: bool,
    ) -> Result<()>;
    async fn mark_conversation_read(&self, conv_id: Uuid) -> Result<()>;

    async fn add_group_member(
        &self,
        conv_id: Uuid,
        contact_id: Uuid,
        role: GroupRole,
        pubkey_snapshot: Option<String>,
    ) -> Result<()>;
    async fn remove_group_member(&self, conv_id: Uuid, contact_id: Uuid) -> Result<()>;
    async fn delete_conversation(&self, id: Uuid) -> Result<()>;

    // ── Ключи диалогов ───────────────────────────────────────────────────────

    async fn create_conversation_keys(
        &self,
        conversation_id: Uuid,
        my_keypair_json: String,
    ) -> Result<()>;
    async fn get_conversation_keys(&self, conversation_id: Uuid) -> Result<ConversationKeys>;
    async fn set_conversation_their_public_key(
        &self,
        conversation_id: Uuid,
        their_public_key_json: String,
    ) -> Result<()>;
    async fn are_conversation_keys_active(&self, conversation_id: Uuid) -> Result<bool>;

    // ── Сообщения ────────────────────────────────────────────────────────────

    async fn create_message(&self, data: CreateMessage) -> Result<()>;
    async fn message_exists(&self, id: Uuid, account_id: Option<Uuid>) -> Result<bool>;
    async fn get_message_history(
        &self,
        conv_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: usize,
    ) -> Result<Vec<Message>>;
    async fn update_message_status(&self, id: Uuid, status: MessageStatus) -> Result<()>;
    async fn update_message_status_with_error(
        &self,
        id: Uuid,
        status: MessageStatus,
        error: Option<String>,
    ) -> Result<()>;
    async fn get_imap_uids_for_deletion(&self, conv_id: Uuid) -> Result<Vec<ImapUidEntry>>;
    async fn delete_conversation_messages(&self, conv_id: Uuid) -> Result<()>;
    async fn get_queued_messages(&self, account_id: Uuid) -> Result<Vec<Message>>;
}

pub type DynStorage = Arc<dyn StoragePort>;
