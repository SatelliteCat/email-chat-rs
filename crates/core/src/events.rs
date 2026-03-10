//! ChatEvent — события которые SyncEngine отправляет в UI.
//!
//! UI подписывается на `tokio::sync::broadcast` канал и
//! реагирует на события: новое сообщение, смена статуса и т.д.
//!
//! ```text
//! SyncEngine
//!     │
//!     └─► EventBus (broadcast::Sender<ChatEvent>)
//!               │
//!               ├─► UI (desktop egui re-render)
//!               └─► UI (mobile push notification)
//! ```

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::models::{
    contact::ContactStatus,
    message::{Message, MessageStatus},
};

/// Все события которые ядро отправляет наружу.
#[derive(Debug, Clone)]
pub enum ChatEvent {
    /// Новое входящее сообщение
    NewMessage {
        conversation_id: Uuid,
        message: Message,
    },

    /// Статус исходящего сообщения изменился
    MessageStatusChanged {
        message_id: Uuid,
        new_status: MessageStatus,
    },

    /// Контакт завершил handshake — канал установлен
    ContactActivated {
        contact_id: Uuid,
        email: String,
    },

    /// Статус контакта изменился
    ContactStatusChanged {
        contact_id: Uuid,
        new_status: ContactStatus,
    },

    /// Новый участник добавлен в группу
    GroupMemberAdded {
        conversation_id: Uuid,
        contact_id: Uuid,
    },

    /// Участник удалён из группы
    GroupMemberRemoved {
        conversation_id: Uuid,
        contact_id: Uuid,
    },

    /// SyncEngine подключился/отключился
    SyncStateChanged { connected: bool },

    /// Ошибка синхронизации (для показа в UI)
    SyncError { message: String },
}

/// Шина событий — обёртка над tokio broadcast.
#[derive(Clone)]
pub struct EventBus {
    sender: tokio::sync::broadcast::Sender<ChatEvent>,
}

impl EventBus {
    /// Создаёт новую шину с заданной ёмкостью буфера.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = tokio::sync::broadcast::channel(capacity);
        Self { sender }
    }

    /// Отправляет событие всем подписчикам.
    /// Если подписчиков нет — игнорируем ошибку.
    pub fn emit(&self, event: ChatEvent) {
        let _ = self.sender.send(event);
    }

    /// Подписывается на события.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<ChatEvent> {
        self.sender.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(256)
    }
}
