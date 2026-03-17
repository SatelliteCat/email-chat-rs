//! Модели данных — Rust-структуры соответствующие таблицам БД.
//!
//! Каждая структура — это строка таблицы. Используют `sqlx::FromRow`
//! для автоматического маппинга из запросов.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Account ───────────────────────────────────────────────────────────────────

/// Строка таблицы `accounts`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AccountRow {
    pub id: String,
    pub email: String,
    pub provider: String,
    pub imap_host: String,
    pub imap_port: i64,
    pub smtp_host: String,
    pub smtp_port: i64,
    pub echat_folder: String,
    pub last_imap_uid: Option<i64>,
    pub last_sync_at: Option<String>,
    pub is_active: i64,
    pub created_at: String,
}

/// Для создания нового аккаунта.
#[derive(Debug, Clone)]
pub struct NewAccount {
    pub id: Uuid,
    pub email: String,
    pub provider: Provider,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub echat_folder: String,
}

// ── Contact ───────────────────────────────────────────────────────────────────

/// Строка таблицы `contacts`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ContactRow {
    pub id: String,
    #[sqlx(default)]
    pub account_id: String,
    #[sqlx(default)]
    pub name: String,
    #[sqlx(default)]
    pub email: String,
    #[sqlx(default)]
    pub avatar: Option<Vec<u8>>,
    #[sqlx(default)]
    pub status: String,
    #[sqlx(default)]
    pub public_keys_json: Option<String>,
    #[sqlx(default)]
    pub handshake_at: Option<String>,
    #[sqlx(default)]
    pub created_at: String,
    #[sqlx(default)]
    pub updated_at: String,
}

/// Для создания нового контакта.
#[derive(Debug, Clone)]
pub struct NewContact {
    pub id: Uuid,
    pub account_id: Uuid,
    pub name: String,
    pub email: String,
    pub avatar: Option<Vec<u8>>,
}

/// Для обновления контакта.
#[derive(Debug, Clone, Default)]
pub struct UpdateContact {
    pub name: Option<String>,
    pub avatar: Option<Option<Vec<u8>>>, // Some(None) — удалить аватар
    pub status: Option<ContactStatus>,
    pub public_keys_json: Option<String>,
}

// ── Conversation ──────────────────────────────────────────────────────────────

/// Строка таблицы `conversations`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ConversationRow {
    pub id: String,
    #[sqlx(default)]
    pub account_id: String,
    #[sqlx(default)]
    pub kind: String,
    #[sqlx(default)]
    pub contact_id: Option<String>,
    #[sqlx(default)]
    pub group_name: Option<String>,
    #[sqlx(default)]
    pub group_avatar: Option<Vec<u8>>,
    #[sqlx(default)]
    pub last_msg_at: Option<String>,
    #[sqlx(default)]
    pub last_msg_preview: Option<String>,
    pub unread_count: i64,
    #[sqlx(default)]
    pub created_at: String,
    #[sqlx(default)]
    pub updated_at: String,
}

/// Строка таблицы `group_members`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GroupMemberRow {
    #[sqlx(default)]
    pub conversation_id: String,
    #[sqlx(default)]
    pub contact_id: String,
    #[sqlx(default)]
    pub role: String,
    #[sqlx(default)]
    pub joined_at: String,
    #[sqlx(default)]
    pub public_key_snapshot: Option<String>,
}

/// Для создания direct-беседы.
#[derive(Debug, Clone)]
pub struct NewDirectConversation {
    pub id: Uuid,
    pub account_id: Uuid,
    pub contact_id: Uuid,
}

/// Для создания группового чата.
#[derive(Debug, Clone)]
pub struct NewGroupConversation {
    pub id: Uuid,
    pub account_id: Uuid,
    pub name: String,
    pub avatar: Option<Vec<u8>>,
    pub members: Vec<NewGroupMember>,
}

#[derive(Debug, Clone)]
pub struct NewGroupMember {
    pub contact_id: Uuid,
    pub role: GroupRole,
    pub public_key_snapshot: Option<String>,
}

// ── Message ───────────────────────────────────────────────────────────────────

/// Строка таблицы `messages`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MessageRow {
    pub id: String,
    pub conversation_id: String,
    pub account_id: String,
    pub from_email: String,
    #[sqlx(default)]
    pub body: Option<String>,
    pub kind: String,
    pub status: String,
    #[sqlx(default)]
    pub reply_to_id: Option<String>,
    #[sqlx(default)]
    pub imap_uid: Option<i64>,
    #[sqlx(default)]
    pub imap_folder: Option<String>,
    pub sent_at: String,
    #[sqlx(default)]
    pub received_at: Option<String>,
    #[sqlx(default)]
    pub error_message: Option<String>,
    pub created_at: String,
}

/// Для вставки нового сообщения.
#[derive(Debug, Clone)]
pub struct NewMessage {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub account_id: Uuid,
    pub from_email: String,
    pub body: Option<String>,
    pub kind: MessageKind,
    pub status: MessageStatus,
    pub reply_to: Option<Uuid>,
    pub reply_to_account_id: Option<Uuid>,
    pub imap_uid: Option<u32>,
    pub imap_folder: Option<String>,
    pub sent_at: DateTime<Utc>,
    pub error_message: Option<String>,
}

impl NewMessage {
    pub fn reply_to_account_id(&self) -> Option<Uuid> {
        // reply_to всегда принадлежит тому же аккаунту что и сообщение
        self.reply_to.map(|_| self.account_id)
    }
}

/// Запись UID для удаления с IMAP сервера.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ImapUidRecord {
    #[sqlx(default)]
    pub imap_uid: Option<i64>,
    #[sqlx(default)]
    pub imap_folder: Option<String>,
}

// ── Conversation Keys ─────────────────────────────────────────────────────────

/// Строка таблицы `conversation_keys`.
#[derive(Debug, Clone, sqlx::FromRow, Default)]
pub struct ConversationKeyRow {
    pub conversation_id: String,
    #[sqlx(default)]
    pub my_keypair_json: Option<String>,
    #[sqlx(default)]
    pub their_public_key_json: Option<String>,
    #[sqlx(default)]
    pub status: String,
    #[sqlx(default)]
    pub created_at: String,
    #[sqlx(default)]
    pub updated_at: String,
}

/// Для создания записи ключей диалога.
#[derive(Debug, Clone)]
pub struct NewConversationKeys {
    pub conversation_id: Uuid,
    pub my_keypair_json: String,
}

/// Для обновления ключей диалога.
#[derive(Debug, Clone, Default)]
pub struct UpdateConversationKeys {
    pub my_keypair_json: Option<String>,
    pub their_public_key_json: Option<String>,
    pub status: Option<ConversationKeyStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConversationKeyStatus {
    Incomplete,
    Active,
}

impl ConversationKeyStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConversationKeyStatus::Incomplete => "incomplete",
            ConversationKeyStatus::Active => "active",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "incomplete" => Some(ConversationKeyStatus::Incomplete),
            "active" => Some(ConversationKeyStatus::Active),
            _ => None,
        }
    }
}

// ── Перечисления ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Gmail,
    MailRu,
    Yandex,
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::Gmail => "gmail",
            Provider::MailRu => "mailru",
            Provider::Yandex => "yandex",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "gmail" => Some(Provider::Gmail),
            "mailru" => Some(Provider::MailRu),
            "yandex" => Some(Provider::Yandex),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContactStatus {
    /// Публичного ключа нет
    NoKey,
    /// Публичный ключ сохранён
    HasKey,
}

impl ContactStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContactStatus::NoKey => "nokey",
            ContactStatus::HasKey => "haskey",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "nokey" => Some(ContactStatus::NoKey),
            "haskey" => Some(ContactStatus::HasKey),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupRole {
    Owner,
    Admin,
    Member,
}

impl GroupRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            GroupRole::Owner => "owner",
            GroupRole::Admin => "admin",
            GroupRole::Member => "member",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageKind {
    Text,
    Handshake,
    GroupEvent,
}

impl MessageKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            MessageKind::Text => "text",
            MessageKind::Handshake => "handshake",
            MessageKind::GroupEvent => "group_event",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageStatus {
    Queued,
    Sending,
    Sent,
    Delivered,
    Read,
    Failed,
}

impl MessageStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            MessageStatus::Queued => "queued",
            MessageStatus::Sending => "sending",
            MessageStatus::Sent => "sent",
            MessageStatus::Delivered => "delivered",
            MessageStatus::Read => "read",
            MessageStatus::Failed => "failed",
        }
    }
}

// ── Утилиты конвертации времени ───────────────────────────────────────────────

pub fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

pub fn parse_dt(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}
