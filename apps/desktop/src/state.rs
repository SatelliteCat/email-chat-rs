//! UiState — состояние интерфейса.
//!
//! Разделено на два уровня:
//! - `Screen` — какой главный экран сейчас показан
//! - `ChatUiState` — детали открытого чата (скролл, черновик и т.д.)

use chrono::{DateTime, Utc};
use uuid::Uuid;

use echat_core::models::{
    account::Account, contact::Contact, conversation::Conversation, message::Message,
};

/// Главный экран приложения.
#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    /// Нет аккаунта — экран добавления
    Login,
    /// Основной экран: список бесед + чат
    Main,
    /// Управление контактами
    Contacts,
    /// Создание нового группового чата
    NewGroup,
}

impl Default for Screen {
    fn default() -> Self {
        Screen::Login
    }
}

/// Всё UI-состояние приложения.
#[derive(Default)]
pub struct UiState {
    pub screen: Screen,

    // ── Аккаунт ──────────────────────────────────────────────────────────────
    /// Активный аккаунт (после логина)
    pub account: Option<Account>,

    // ── Login форма ───────────────────────────────────────────────────────────
    pub login: LoginState,

    // ── Список бесед ─────────────────────────────────────────────────────────
    pub conversations: Vec<ConversationItem>,
    /// ID выбранной беседы
    pub selected_conv_id: Option<Uuid>,
    pub sidebar_search: String,

    // ── Ожидает открытия чата после создания беседы ─────────────────────────
    pub pending_chat_open: Option<(Uuid, Uuid)>, // (conv_id, contact_id)

    // ── Открытый чат ─────────────────────────────────────────────────────────
    pub chat: ChatUiState,

    // ── Контакты ─────────────────────────────────────────────────────────────
    pub contacts: Vec<Contact>,
    pub contact_search: String,
    pub new_contact_email: String,
    pub new_contact_name: String,

    // ── Новая группа ─────────────────────────────────────────────────────────
    pub new_group_name: String,
    pub new_group_selected: Vec<Uuid>, // contact_id'ы

    // ── Уведомления ──────────────────────────────────────────────────────────
    pub toasts: Vec<Toast>,

    // ── Статус синхронизации ─────────────────────────────────────────────────
    pub sync_connected: bool,
    pub sync_error: Option<String>,
    /// Флаг для принудительной синхронизации
    pub force_sync: bool,
}

impl UiState {
    /// Возвращает выбранную беседу.
    pub fn selected_conversation(&self) -> Option<&ConversationItem> {
        let id = self.selected_conv_id?;
        self.conversations.iter().find(|c| c.id == id)
    }

    /// Добавляет toast-уведомление.
    pub fn toast_info(&mut self, msg: impl Into<String>) {
        self.toasts.push(Toast {
            message: msg.into(),
            kind: ToastKind::Info,
            shown_at: Utc::now(),
        });
    }

    pub fn toast_error(&mut self, msg: impl Into<String>) {
        self.toasts.push(Toast {
            message: msg.into(),
            kind: ToastKind::Error,
            shown_at: Utc::now(),
        });
    }

    /// Убирает устаревшие toasts (старше 4 секунд).
    pub fn expire_toasts(&mut self) {
        let now = Utc::now();
        self.toasts.retain(|t| (now - t.shown_at).num_seconds() < 4);
    }

    /// Добавляет или обновляет сообщение в открытом чате.
    pub fn push_message(&mut self, conv_id: Uuid, msg: Message) {
        // Обновляем список бесед
        if let Some(conv) = self.conversations.iter_mut().find(|c| c.id == conv_id) {
            conv.last_preview = msg.body.clone().unwrap_or_default();
            conv.last_msg_at = Some(msg.sent_at);
            if self.selected_conv_id != Some(conv_id) {
                conv.unread += 1;
            }
        }

        // Если беседа открыта — добавляем сообщение
        if self.selected_conv_id == Some(conv_id) {
            // Дедупликация
            if !self.chat.messages.iter().any(|m| m.id == msg.id) {
                self.chat.messages.push(msg);
                self.chat.scroll_to_bottom = true;
            }
        }
    }
}

// ── Вложенные состояния ───────────────────────────────────────────────────────

#[derive(Default)]
pub struct LoginState {
    pub email: String,
    pub password: String,
    pub show_password: bool,
    pub is_loading: bool,
    pub error: Option<String>,
    /// Идёт процесс автоматического входа (не показывать ошибки)
    pub is_auto_login: bool,
}

/// Элемент списка бесед в сайдбаре.
#[derive(Clone)]
pub struct ConversationItem {
    pub id: Uuid,
    pub display_name: String,
    pub last_preview: String,
    pub last_msg_at: Option<DateTime<Utc>>,
    pub unread: u32,
    pub avatar_letter: char, // первая буква имени для аватара-заглушки
}

impl ConversationItem {
    pub fn from_conversation(conv: &Conversation, display_name: &str) -> Self {
        Self {
            id: conv.id,
            display_name: display_name.to_string(),
            last_preview: conv.last_msg_preview.clone().unwrap_or_default(),
            last_msg_at: conv.last_msg_at,
            unread: conv.unread_count,
            avatar_letter: display_name
                .chars()
                .next()
                .unwrap_or('?')
                .to_uppercase()
                .next()
                .unwrap_or('?'),
        }
    }
}

pub struct ChatUiState {
    /// Загруженные сообщения (в порядке отправки, старые первые)
    pub messages: Vec<Message>,
    /// Черновик сообщения в поле ввода
    pub draft: String,
    /// Прокрутить вниз при следующем кадре
    pub scroll_to_bottom: bool,
    /// ID беседы для которой загружены сообщения
    pub loaded_conv_id: Option<Uuid>,
    /// Загрузка истории в процессе
    pub is_loading_history: bool,
    /// Публичный ключ нашего аккаунта для этого диалога (для копирования)
    pub my_public_key: String,
    /// Поле для вставки публичного ключа собеседника
    pub their_public_key_input: String,
    /// Сообщение о статусе ключей
    pub keys_status_message: Option<String>,
}

impl Default for ChatUiState {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            draft: String::new(),
            scroll_to_bottom: false,
            loaded_conv_id: None,
            is_loading_history: false,
            my_public_key: String::new(),
            their_public_key_input: String::new(),
            keys_status_message: None,
        }
    }
}

#[derive(Clone)]
pub struct Toast {
    pub message: String,
    pub kind: ToastKind,
    pub shown_at: DateTime<Utc>,
}

#[derive(Clone, PartialEq)]
pub enum ToastKind {
    Info,
    Error,
}
