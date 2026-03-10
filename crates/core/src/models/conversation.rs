use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: Uuid,
    pub account_id: Uuid,
    pub kind: ConversationKind,
    pub last_msg_at: Option<DateTime<Utc>>,
    pub last_msg_preview: Option<String>,
    pub unread_count: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConversationKind {
    Direct { contact_id: Uuid },
    Group {
        name: String,
        avatar: Option<Vec<u8>>,
        /// Загружается отдельно через get_group_members
        members: Vec<GroupMember>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMember {
    pub contact_id: Uuid,
    pub role: GroupRole,
    pub joined_at: DateTime<Utc>,
    /// Снапшот публичного ключа на момент добавления в группу
    pub public_key_snapshot: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupRole {
    Owner,
    Admin,
    Member,
}

impl GroupRole {
    pub fn can_add_members(&self) -> bool {
        matches!(self, GroupRole::Owner | GroupRole::Admin)
    }

    pub fn can_remove_members(&self) -> bool {
        matches!(self, GroupRole::Owner | GroupRole::Admin)
    }
}

impl std::fmt::Display for GroupRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GroupRole::Owner => write!(f, "Владелец"),
            GroupRole::Admin => write!(f, "Администратор"),
            GroupRole::Member => write!(f, "Участник"),
        }
    }
}
