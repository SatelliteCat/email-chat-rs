//! Runtime — мост между async tokio и sync egui.
//!
//! ```text
//! egui update()
//!     ├─► spawn(async { ... })   →  tokio thread pool
//!     └─► poll_events()          ←  mpsc::Receiver<AppEvent>
//!                └─► UiState     →  ctx.request_repaint()
//! ```

use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

use echat_core::{
    AppState, ChatEvent,
    models::{account::Account, contact::Contact, conversation::Conversation, message::Message},
    services::history_restorer::RestoreStats,
};

/// Все события из async-задач → UI.
pub enum AppEvent {
    // ── Логин ─────────────────────────────────────────────────────────────
    LoginAttempt {
        email: String,
        password: String,
    },
    /// AppState успешно собран, аккаунт готов
    AccountReady {
        state: AppState,
        account: Account,
    },
    AccountError(String),
    /// Автоматический вход завершён (нет аккаунтов или ошибка)
    AutoLoginComplete,
    /// Пользователь нажал выход
    Logout,
    /// Выход завершён
    LogoutComplete,

    // ── Данные ────────────────────────────────────────────────────────────
    ConversationsLoaded(Vec<Conversation>),
    ContactsLoaded(Vec<Contact>),
    HistoryLoaded {
        conv_id: Uuid,
        messages: Vec<Message>,
    },

    // ── Сообщения ─────────────────────────────────────────────────────────
    NewMessage {
        conv_id: Uuid,
        message: Message,
    },
    MessageSent {
        conv_id: Uuid,
        message: Message,
    },
    SendError(String),

    // ── Контакты ──────────────────────────────────────────────────────────
    AddContact {
        email: String,
        name: String,
    },
    ContactAdded,
    ContactError(String),
    OpenChatWith {
        contact_id: Uuid,
    },
    /// Беседа создана, нужно открыть чат
    ChatCreated {
        conv_id: Uuid,
        contact_id: Uuid,
    },
    DeleteContact {
        contact_id: Uuid,
    },
    ContactDeleted {
        contact_id: Uuid,
    },
    ContactActivated {
        contact_id: Uuid,
        email: String,
    },

    // ── Ключи диалогов ────────────────────────────────────────────────────
    /// Загрузить ключи диалога
    LoadConversationKeys {
        conv_id: Uuid,
    },
    /// Ключи диалога загружены
    ConversationKeysLoaded {
        conv_id: Uuid,
        my_public_key: String,
        their_public_key: Option<String>,
        is_active: bool,
    },
    /// Установить публичный ключ собеседника
    SetTheirPublicKey {
        conv_id: Uuid,
        public_key_json: String,
    },
    /// Ключ собеседника установлен
    TheirPublicKeySet {
        conv_id: Uuid,
    },
    /// Импортировать seed нашего ключа для восстановления истории
    ImportMyKeypairSeed {
        conv_id: Uuid,
        seed_base64: String,
    },
    /// Seed нашего ключа импортирован
    MyKeypairSeedImported {
        conv_id: Uuid,
    },
    KeysError(String),

    // ── Беседы ────────────────────────────────────────────────────────────
    DeleteConversation {
        conv_id: Uuid,
    },
    ConversationDeleted {
        conv_id: Uuid,
    },

    // ── Синхронизация ─────────────────────────────────────────────────────
    SyncConnected(bool),
    SyncError(String),

    // ── Восстановление истории ────────────────────────────────────────────
    HistoryRestoreComplete {
        stats: RestoreStats,
    },
}

// AppState не реализует Debug автоматически — обходим вручную
impl std::fmt::Debug for AppEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppEvent::AccountReady { account, .. } => write!(f, "AccountReady({})", account.email),
            AppEvent::LoginAttempt { email, .. } => write!(f, "LoginAttempt({})", email),
            AppEvent::ConversationsLoaded(v) => write!(f, "ConversationsLoaded({})", v.len()),
            AppEvent::ContactsLoaded(v) => write!(f, "ContactsLoaded({})", v.len()),
            AppEvent::HistoryLoaded { conv_id, messages } => {
                write!(f, "HistoryLoaded({}, {} msgs)", conv_id, messages.len())
            }
            AppEvent::NewMessage { conv_id, .. } => write!(f, "NewMessage(conv={})", conv_id),
            AppEvent::MessageSent { conv_id, .. } => write!(f, "MessageSent(conv={})", conv_id),
            AppEvent::HistoryRestoreComplete { stats } => {
                write!(
                    f,
                    "HistoryRestoreComplete(decrypted={}, encrypted={})",
                    stats.decrypted, stats.encrypted_stored
                )
            }
            other => write!(f, "{:?}", std::mem::discriminant(other)),
        }
    }
}

/// Мост egui ↔ tokio.
pub struct AsyncRuntime {
    rt: Arc<tokio::runtime::Runtime>,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    pub event_rx: mpsc::UnboundedReceiver<AppEvent>,
    ctx: Option<egui::Context>,
}

impl AsyncRuntime {
    pub fn new() -> Self {
        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("echat-async")
                .build()
                .expect("tokio runtime"),
        );
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        Self {
            rt,
            event_tx,
            event_rx,
            ctx: None,
        }
    }

    pub fn set_ctx(&mut self, ctx: egui::Context) {
        self.ctx = Some(ctx);
    }

    pub fn spawn<F>(&self, task: F) -> tokio::task::JoinHandle<()>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        self.rt.spawn(task)
    }

    pub fn event_sender(&self) -> EventSender {
        EventSender {
            tx: self.event_tx.clone(),
            ctx: self.ctx.clone(),
        }
    }

    pub fn rt(&self) -> Arc<tokio::runtime::Runtime> {
        self.rt.clone()
    }
}

/// Cloneable sender — передаётся в async задачи и view-компоненты.
#[derive(Clone)]
pub struct EventSender {
    tx: mpsc::UnboundedSender<AppEvent>,
    ctx: Option<egui::Context>,
}

impl EventSender {
    pub fn send(&self, event: AppEvent) {
        let _ = self.tx.send(event);
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint();
        }
    }
}

/// Слушает EventBus ядра и пересылает нужные события в UI.
pub fn subscribe_to_core_events(
    event_bus: echat_core::EventBus,
    sender: EventSender,
    rt: Arc<tokio::runtime::Runtime>,
) {
    rt.spawn(async move {
        let mut rx = event_bus.subscribe();
        loop {
            match rx.recv().await {
                Ok(ChatEvent::NewMessage {
                    conversation_id,
                    message,
                }) => {
                    sender.send(AppEvent::NewMessage {
                        conv_id: conversation_id,
                        message,
                    });
                }
                Ok(ChatEvent::SyncStateChanged { connected }) => {
                    sender.send(AppEvent::SyncConnected(connected));
                }
                Ok(ChatEvent::SyncError { message }) => {
                    sender.send(AppEvent::SyncError(message));
                }
                Ok(ChatEvent::ContactActivated { contact_id, email }) => {
                    sender.send(AppEvent::ContactActivated { contact_id, email });
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("EventBus: пропущено {} событий", n);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}
