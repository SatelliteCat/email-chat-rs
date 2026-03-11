//! EchatApp — реализует `eframe::App`.
//!
//! Главный цикл update() делает три вещи:
//!  1. Вычитывает AppEvent из async канала → применяет к UiState
//!  2. Рисует нужный Screen
//!  3. Запускает async задачи в ответ на действия пользователя

use std::sync::Arc;

use egui::{CentralPanel, Context, Frame, SidePanel, TopBottomPanel};
use uuid::Uuid;

use core::{AppConfig, AppState};

use crate::{
    runtime::{subscribe_to_core_events, AppEvent, AsyncRuntime, EventSender},
    state::{ConversationItem, Screen, UiState},
    views::{chat, compose, contacts, login, sidebar, theme},
};

pub struct EchatApp {
    rt: AsyncRuntime,
    /// Заполняется после успешного логина
    app_state: Option<Arc<AppState>>,
    /// ID аккаунта из БД
    account_id: Option<Uuid>,
    ui: UiState,
}

impl EchatApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);

        let mut rt = AsyncRuntime::new();
        rt.set_ctx(cc.egui_ctx.clone());

        Self {
            rt,
            app_state: None,
            account_id: None,
            ui: UiState::default(),
        }
    }
}

impl eframe::App for EchatApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.poll_events();
        self.ui.expire_toasts();

        match self.ui.screen.clone() {
            Screen::Login    => self.draw_login(ctx),
            Screen::Main     => self.draw_main(ctx),
            Screen::Contacts => self.draw_contacts(ctx),
            Screen::NewGroup => { /* TODO */ }
        }

        self.draw_toasts(ctx);
    }
}

// ── Обработка событий ─────────────────────────────────────────────────────────

impl EchatApp {
    fn poll_events(&mut self) {
        while let Ok(event) = self.rt.event_rx.try_recv() {
            self.handle_event(event);
        }
    }

    fn handle_event(&mut self, event: AppEvent) {
        match event {
            // ── Логин ─────────────────────────────────────────────────────
            AppEvent::LoginAttempt { email, password } => {
                self.spawn_login(email, password);
            }

            AppEvent::AccountReady { state, account } => {
                let event_bus = state.events.clone();
                self.app_state = Some(Arc::new(state));
                self.account_id = Some(account.id);
                self.ui.account = Some(account);
                self.ui.login.is_loading = false;
                self.ui.screen = Screen::Main;

                // Подписываемся на события ядра
                subscribe_to_core_events(event_bus, self.rt.event_sender(), self.rt.rt());

                self.load_conversations();
                self.load_contacts();
            }

            AppEvent::AccountError(e) => {
                self.ui.login.is_loading = false;
                self.ui.login.error = Some(e);
            }

            // ── Беседы ────────────────────────────────────────────────────
            AppEvent::ConversationsLoaded(convs) => {
                let contacts = &self.ui.contacts;
                self.ui.conversations = convs
                    .iter()
                    .map(|c| {
                        let name = match &c.kind {
                            core::models::conversation::ConversationKind::Direct {
                                contact_id,
                            } => contacts
                                .iter()
                                .find(|ct| &ct.id == contact_id)
                                .map(|ct| ct.name.clone())
                                .unwrap_or_else(|| "Неизвестный".to_string()),
                            core::models::conversation::ConversationKind::Group {
                                name, ..
                            } => name.clone(),
                        };
                        ConversationItem::from_conversation(c, &name)
                    })
                    .collect();
            }

            AppEvent::ContactsLoaded(contacts) => {
                self.ui.contacts = contacts;
            }

            AppEvent::HistoryLoaded { conv_id, messages } => {
                if self.ui.selected_conv_id == Some(conv_id) {
                    self.ui.chat.messages = messages;
                    self.ui.chat.loaded_conv_id = Some(conv_id);
                    self.ui.chat.is_loading_history = false;
                    self.ui.chat.scroll_to_bottom = true;
                }
            }

            // ── Сообщения ─────────────────────────────────────────────────
            AppEvent::NewMessage { conv_id, message } => {
                self.ui.push_message(conv_id, message);
            }

            AppEvent::MessageSent { conv_id, message } => {
                self.ui.push_message(conv_id, message);
            }

            AppEvent::SendError(e) => {
                self.ui.toast_error(format!("Ошибка отправки: {}", e));
            }

            // ── Контакты ──────────────────────────────────────────────────
            AppEvent::AddContact { email, name } => {
                self.spawn_add_contact(email, name);
            }

            AppEvent::ContactAdded => {
                self.ui.toast_info("Контакт добавлен, отправляем handshake…");
                self.load_contacts();
            }

            AppEvent::ContactError(e) => {
                self.ui.toast_error(format!("Ошибка: {}", e));
            }

            AppEvent::OpenChatWith { contact_id } => {
                self.open_chat_with(contact_id);
            }

            // ── Синхронизация ─────────────────────────────────────────────
            AppEvent::SyncConnected(c) => {
                self.ui.sync_connected = c;
            }

            AppEvent::SyncError(e) => {
                tracing::warn!("Sync error: {}", e);
                self.ui.sync_error = Some(e);
            }

            AppEvent::DeleteConversation { conv_id } => {
                self.spawn_delete_conversation(conv_id);
            }

            AppEvent::ConversationDeleted { conv_id } => {
                self.ui.conversations.retain(|c| c.id != conv_id);
                if self.ui.selected_conv_id == Some(conv_id) {
                    self.ui.selected_conv_id = None;
                    self.ui.chat = Default::default();
                }
                self.ui.toast_info("Беседа удалена");
            }
        }
    }
}

// ── Отрисовка ─────────────────────────────────────────────────────────────────

impl EchatApp {
    fn draw_login(&mut self, ctx: &Context) {
        let sender = self.rt.event_sender();
        CentralPanel::default()
            .frame(Frame::none().fill(theme::BG_DARK))
            .show(ctx, |ui| {
                login::show(ui, &mut self.ui.login, &sender);
            });
    }

    fn draw_main(&mut self, ctx: &Context) {
        let sender = self.rt.event_sender();
        let my_email = self
            .ui
            .account
            .as_ref()
            .map(|a| a.email.clone())
            .unwrap_or_default();

        // Левая панель — список бесед
        SidePanel::left("sidebar")
            .resizable(true)
            .default_width(290.0)
            .min_width(220.0)
            .max_width(400.0)
            .frame(Frame::none())
            .show(ctx, |ui| {
                if let Some(conv_id) = sidebar::show(ui, &mut self.ui) {
                    self.select_conversation(conv_id);
                }
            });

        // Правая часть — чат
        CentralPanel::default()
            .frame(Frame::none().fill(theme::BG_DARK))
            .show(ctx, |ui| {
                if let Some(conv) = self.ui.selected_conversation().cloned() {
                    // Поле ввода — снизу
                    TopBottomPanel::bottom("compose")
                        .exact_height(58.0)
                        .frame(Frame::none())
                        .show_inside(ui, |ui| {
                            let action =
                                compose::show(ui, &mut self.ui.chat.draft, false);
                            if let compose::ComposeAction::Send(text) = action {
                                self.spawn_send_message(conv.id, text);
                            }
                        });

                    // История сообщений
                    chat::show(ui, &mut self.ui.chat, &conv, &my_email, &sender);
                } else {
                    // Заглушка «выберите беседу»
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new("Выберите беседу или добавьте контакт")
                                .color(theme::TEXT_TIMESTAMP),
                        );
                    });
                }
            });
    }

    fn draw_contacts(&mut self, ctx: &Context) {
        let sender = self.rt.event_sender();
        CentralPanel::default()
            .frame(Frame::none().fill(theme::BG_DARK))
            .show(ctx, |ui| {
                contacts::show(ui, &mut self.ui, &sender);
            });
    }

    fn draw_toasts(&self, ctx: &Context) {
        for (i, toast) in self.ui.toasts.iter().enumerate() {
            let (bg, fg) = match toast.kind {
                crate::state::ToastKind::Info => {
                    (theme::BG_MSG_IN, theme::TEXT_PRIMARY)
                }
                crate::state::ToastKind::Error => {
                    (egui::Color32::from_rgb(60, 20, 20), theme::ERROR)
                }
            };

            egui::Window::new(format!("##toast{}", i))
                .title_bar(false)
                .resizable(false)
                .collapsible(false)
                .anchor(
                    egui::Align2::RIGHT_BOTTOM,
                    egui::vec2(-16.0, -16.0 - i as f32 * 52.0),
                )
                .frame(
                    Frame::none()
                        .fill(bg)
                        .rounding(egui::Rounding::same(8.0))
                        .inner_margin(egui::Margin::symmetric(16.0, 10.0)),
                )
                .show(ctx, |ui| {
                    ui.label(egui::RichText::new(&toast.message).color(fg));
                });
        }
    }
}

// ── Async операции ────────────────────────────────────────────────────────────

impl EchatApp {
    fn spawn_login(&mut self, email: String, password: String) {
        let sender = self.rt.event_sender();
        let db_path = app_db_path();
        let config = AppConfig::default();

        self.rt.spawn(async move {
            // Собираем AppState через platform
            let state =
                match platform::build_app_state(&email, &password, &db_path, config).await {
                    Ok(s) => s,
                    Err(e) => {
                        sender.send(AppEvent::AccountError(format!(
                            "Не удалось подключиться: {}",
                            e
                        )));
                        return;
                    }
                };

            // Определяем провайдера по домену
            let provider = match provider_from_email(&email) {
                Some(p) => p,
                None => {
                    sender.send(AppEvent::AccountError(
                        "Поддерживаются только mail.ru и yandex.ru".into(),
                    ));
                    return;
                }
            };

            // Добавляем аккаунт (или получаем существующий)
            let account = match state
                .account_service
                .add_account(email.clone(), password, provider)
                .await
            {
                Ok(acc) => acc,
                // Уже существует — загружаем
                Err(core::Error::AlreadyExists(_)) => {
                    match state.account_service.list_accounts().await {
                        Ok(mut list) if !list.is_empty() => list.remove(0),
                        _ => {
                            sender.send(AppEvent::AccountError(
                                "Не удалось загрузить аккаунт".into(),
                            ));
                            return;
                        }
                    }
                }
                Err(e) => {
                    sender.send(AppEvent::AccountError(e.to_string()));
                    return;
                }
            };

            sender.send(AppEvent::AccountReady { state, account });
        });
    }

    fn load_conversations(&mut self) {
        let (state, account_id) = match self.services() {
            Some(x) => x,
            None => return,
        };
        let sender = self.rt.event_sender();

        self.rt.spawn(async move {
            match state.chat_service.list_conversations(account_id).await {
                Ok(convs) => sender.send(AppEvent::ConversationsLoaded(convs)),
                Err(e) => tracing::warn!("Ошибка загрузки бесед: {}", e),
            }
        });
    }

    fn load_contacts(&mut self) {
        let (state, account_id) = match self.services() {
            Some(x) => x,
            None => return,
        };
        let sender = self.rt.event_sender();

        self.rt.spawn(async move {
            match state.contact_service.list_contacts(account_id).await {
                Ok(contacts) => sender.send(AppEvent::ContactsLoaded(contacts)),
                Err(e) => tracing::warn!("Ошибка загрузки контактов: {}", e),
            }
        });
    }

    fn select_conversation(&mut self, conv_id: Uuid) {
        if self.ui.selected_conv_id == Some(conv_id) {
            return;
        }
        self.ui.selected_conv_id = Some(conv_id);
        self.ui.chat = Default::default();
        self.ui.chat.is_loading_history = true;

        // Сбрасываем счётчик непрочитанных
        if let Some(c) = self.ui.conversations.iter_mut().find(|c| c.id == conv_id) {
            c.unread = 0;
        }

        self.load_history(conv_id);
    }

    fn load_history(&mut self, conv_id: Uuid) {
        let (state, _) = match self.services() {
            Some(x) => x,
            None => return,
        };
        let sender = self.rt.event_sender();

        self.rt.spawn(async move {
            match state.chat_service.get_history(conv_id, None, 60).await {
                Ok(msgs) => sender.send(AppEvent::HistoryLoaded { conv_id, messages: msgs }),
                Err(e) => tracing::warn!("Ошибка загрузки истории: {}", e),
            }
        });
    }

    fn spawn_send_message(&mut self, conv_id: Uuid, text: String) {
        let (state, account_id) = match self.services() {
            Some(x) => x,
            None => return,
        };
        let sender = self.rt.event_sender();

        // Берём contact_id из беседы
        let contact_id = self.contact_id_for_conv(conv_id);
        let Some(contact_id) = contact_id else {
            self.ui.toast_error("Не удалось определить получателя");
            return;
        };

        self.rt.spawn(async move {
            match state
                .chat_service
                .send_message(account_id, contact_id, text, None)
                .await
            {
                Ok(msg) => sender.send(AppEvent::MessageSent { conv_id, message: msg }),
                Err(e) => sender.send(AppEvent::SendError(e.to_string())),
            }
        });
    }

    fn spawn_add_contact(&mut self, email: String, name: String) {
        let (state, account_id) = match self.services() {
            Some(x) => x,
            None => return,
        };
        let sender = self.rt.event_sender();

        self.rt.spawn(async move {
            match state
                .contact_service
                .add_contact(account_id, name, email, None)
                .await
            {
                Ok(_) => sender.send(AppEvent::ContactAdded),
                Err(e) => sender.send(AppEvent::ContactError(e.to_string())),
            }
        });
    }

    fn open_chat_with(&mut self, contact_id: Uuid) {
        // Ищем уже существующую direct-беседу
        let existing = self.ui.contacts.iter().find(|c| c.id == contact_id).map(|c| {
            self.ui
                .conversations
                .iter()
                .find(|conv| {
                    // Используем display_name как прокси (до полного Conversation хранения)
                    conv.display_name == c.name || conv.display_name == c.email
                })
                .map(|c| c.id)
        });

        if let Some(Some(conv_id)) = existing {
            self.select_conversation(conv_id);
            self.ui.screen = Screen::Main;
        } else {
            // Беседы нет — создадим при первой отправке
            self.ui.screen = Screen::Main;
        }
    }

    fn spawn_delete_conversation(&mut self, conv_id: Uuid) {
        let (state, _) = match self.services() {
            Some(x) => x,
            None => return,
        };
        let sender = self.rt.event_sender();

        self.rt.spawn(async move {
            match state.chat_service.delete_conversation(conv_id, true).await {
                Ok(()) => sender.send(AppEvent::ConversationDeleted { conv_id }),
                Err(e) => sender.send(AppEvent::SendError(e.to_string())),
            }
        });
    }

    // ── Вспомогательные ───────────────────────────────────────────────────────

    fn services(&self) -> Option<(Arc<AppState>, Uuid)> {
        let state = self.app_state.clone()?;
        let account_id = self.account_id?;
        Some((state, account_id))
    }

    fn contact_id_for_conv(&self, conv_id: Uuid) -> Option<Uuid> {
        // Для direct-беседы contact_id совпадает с именем в contacts
        // Полное решение: хранить ConversationKind в ConversationItem
        // Пока ищем по display_name
        let conv = self.ui.conversations.iter().find(|c| c.id == conv_id)?;
        self.ui
            .contacts
            .iter()
            .find(|c| c.name == conv.display_name || c.email == conv.display_name)
            .map(|c| c.id)
    }
}

// ── Утилиты ───────────────────────────────────────────────────────────────────

fn app_db_path() -> String {
    let dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("echat");
    std::fs::create_dir_all(&dir).ok();
    dir.join("db.sqlite").to_string_lossy().into_owned()
}

fn provider_from_email(email: &str) -> Option<core::models::account::Provider> {
    use core::models::account::Provider;
    let domain = email.split('@').nth(1)?;
    match domain {
        "mail.ru" | "inbox.ru" | "list.ru" | "bk.ru" => Some(Provider::MailRu),
        "yandex.ru" | "ya.ru" | "yandex.com" | "yandex.kz" | "yandex.by" => {
            Some(Provider::Yandex)
        }
        _ => None,
    }
}
