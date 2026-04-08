//! EchatApp — реализует `eframe::App`.
//!
//! Главный цикл update() делает три вещи:
//!  1. Вычитывает AppEvent из async канала → применяет к UiState
//!  2. Рисует нужный Screen
//!  3. Запускает async задачи в ответ на действия пользователя

use std::sync::Arc;

use egui::{CentralPanel, Context, Frame, SidePanel, TopBottomPanel};
use uuid::Uuid;

use echat_core::{AppConfig, AppState, sync::engine::SyncCommand};

use crate::{
    runtime::{AppEvent, AsyncRuntime, EventSender, subscribe_to_core_events},
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
    /// Handle задачи SyncEngine
    _sync_handle: Option<tokio::task::JoinHandle<()>>,
    /// Sender для команд SyncEngine (должен жить пока SyncEngine работает)
    _sync_cmd_tx: Option<tokio::sync::mpsc::Sender<SyncCommand>>,
}

impl EchatApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);

        let mut rt = AsyncRuntime::new();
        rt.set_ctx(cc.egui_ctx.clone());

        let mut app = Self {
            rt,
            app_state: None,
            account_id: None,
            ui: UiState::default(),
            _sync_handle: None,
            _sync_cmd_tx: None,
        };

        // Пробуем автоматически войти с последним аккаунтом
        app.try_auto_login();

        app
    }
}

impl eframe::App for EchatApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.poll_events();
        self.ui.expire_toasts();

        // Проверка флага принудительной синхронизации
        if self.ui.force_sync {
            self.ui.force_sync = false;
            if let Some(ref cmd_tx) = self._sync_cmd_tx {
                let _ = cmd_tx.try_send(echat_core::sync::engine::SyncCommand::FetchNow);
            }
        }

        // Периодическое обновление: каждые 5 секунд подтягиваем сообщения
        // для открытого диалога и каждые 15 секунд — список бесед.
        // Это страхует от потери событий EventBus (broadcast lag).
        if self.ui.screen == Screen::Main {
            if self.ui.should_refresh_chat(5) {
                if let Some(conv_id) = self.ui.selected_conv_id {
                    self.ui.mark_chat_refreshed();
                    self.refresh_current_chat(conv_id);
                }
            }
            if self.ui.should_refresh_conversations(15) {
                self.ui.mark_conversations_refreshed();
                self.load_conversations();
            }
        }

        match self.ui.screen.clone() {
            Screen::Login => self.draw_login(ctx),
            Screen::Main => self.draw_main(ctx),
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

                // Запускаем SyncEngine для получения новых писем
                let (cmd_tx, sync_future) = state.spawn_sync(account.id);
                let sync_handle = self.rt.spawn(sync_future);

                self.app_state = Some(Arc::new(state));
                self.account_id = Some(account.id);
                self.ui.account = Some(account);
                self.ui.login.is_loading = false;
                self.ui.screen = Screen::Main;
                self._sync_handle = Some(sync_handle);
                self._sync_cmd_tx = Some(cmd_tx);

                // Подписываемся на события ядра
                subscribe_to_core_events(event_bus, self.rt.event_sender(), self.rt.rt());

                self.load_conversations();
                self.load_contacts();
            }

            AppEvent::AccountError(e) => {
                // Не показываем ошибки во время авто-входа
                if !self.ui.login.is_auto_login {
                    self.ui.login.is_loading = false;
                    self.ui.login.error = Some(e);
                }
            }

            AppEvent::AutoLoginComplete => {
                // Завершили авто-вход, теперь можно показывать ошибки
                self.ui.login.is_auto_login = false;
            }

            AppEvent::Logout => {
                self.spawn_logout();
            }

            AppEvent::LogoutComplete => {
                // Очищаем всё UI состояние
                self.app_state = None;
                self.account_id = None;
                self.ui = UiState::default();
                self._sync_handle = None;
                self._sync_cmd_tx = None;
                self.ui.screen = Screen::Login;
                self.ui.toast_info("Вы вышли из аккаунта");
            }

            // ── Беседы ────────────────────────────────────────────────────
            AppEvent::ConversationsLoaded(convs) => {
                let contacts = &self.ui.contacts;
                self.ui.conversations = convs
                    .iter()
                    .map(|c| {
                        let name = match &c.kind {
                            echat_core::models::conversation::ConversationKind::Direct {
                                contact_id,
                            } => contacts
                                .iter()
                                .find(|ct| &ct.id == contact_id)
                                .map(|ct| ct.name.clone())
                                .unwrap_or_else(|| "Неизвестный".to_string()),
                            echat_core::models::conversation::ConversationKind::Group {
                                name,
                                ..
                            } => name.clone(),
                        };
                        ConversationItem::from_conversation(c, &name)
                    })
                    .collect();

                // Если есть отложенное открытие чата — открываем его
                if let Some((conv_id, _contact_id)) = self.ui.pending_chat_open.take() {
                    self.select_conversation(conv_id);
                    self.ui.screen = Screen::Main;
                }
            }

            AppEvent::ContactsLoaded(contacts) => {
                self.ui.contacts = contacts;
            }

            AppEvent::HistoryLoaded { conv_id, messages } => {
                if self.ui.selected_conv_id == Some(conv_id) {
                    // Если загруженный список пустой — ничего не делаем.
                    // Если в UI уже есть сообщения (периодический refresh) —
                    // добавляем только новые, иначе — заменяем полностью.
                    if messages.is_empty() {
                        self.ui.chat.is_loading_history = false;
                    } else if self.ui.chat.messages.is_empty()
                        || self.ui.chat.loaded_conv_id != Some(conv_id)
                    {
                        // Первая загрузка — заменяем полностью
                        self.ui.chat.messages = messages;
                        self.ui.chat.loaded_conv_id = Some(conv_id);
                        self.ui.chat.is_loading_history = false;
                        self.ui.chat.scroll_to_bottom = true;
                    } else {
                        // Периодическое обновление — добавляем только новые
                        let existing: std::collections::HashSet<Uuid> =
                            self.ui.chat.messages.iter().map(|m| m.id).collect();
                        let mut added = 0;
                        for msg in messages {
                            if !existing.contains(&msg.id) {
                                self.ui.chat.messages.push(msg);
                                added += 1;
                            }
                        }
                        if added > 0 {
                            // Сортируем по sent_at (старые первые)
                            self.ui.chat.messages.sort_by_key(|m| m.sent_at);
                            self.ui.chat.scroll_to_bottom = true;
                        }
                    }
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
                self.ui
                    .toast_info("Контакт добавлен, отправляем handshake…");
                self.load_contacts();
            }

            AppEvent::ContactError(e) => {
                self.ui.toast_error(format!("Ошибка: {}", e));
            }

            AppEvent::OpenChatWith { contact_id } => {
                self.open_chat_with(contact_id);
            }

            AppEvent::ChatCreated {
                conv_id,
                contact_id,
            } => {
                // Беседа создана — загружаем контакты (для имени) и выбираем беседу
                self.load_contacts();
                self.load_conversations();
                // Дадим UI обновиться, затем выберем беседу
                self.ui.pending_chat_open = Some((conv_id, contact_id));
            }

            AppEvent::DeleteContact { contact_id } => {
                self.spawn_delete_contact(contact_id);
            }

            AppEvent::ContactDeleted { contact_id } => {
                self.ui.contacts.retain(|c| c.id != contact_id);
                self.ui.toast_info("Контакт удалён");
            }

            AppEvent::ContactActivated { email, .. } => {
                self.ui
                    .toast_info(format!("Контакт {} активирован!", email));
                self.load_contacts();
            }

            // ── Ключи диалогов ────────────────────────────────────────────
            AppEvent::LoadConversationKeys { conv_id } => {
                self.spawn_load_conversation_keys(conv_id);
            }

            AppEvent::SetTheirPublicKey {
                conv_id,
                public_key_json,
            } => {
                self.spawn_set_their_public_key(conv_id, public_key_json);
            }

            AppEvent::ConversationKeysLoaded {
                conv_id,
                my_public_key,
                their_public_key,
                is_active,
            } => {
                if self.ui.selected_conv_id == Some(conv_id) {
                    self.ui.chat.my_public_key = my_public_key;
                    if let Some(key) = their_public_key {
                        self.ui.chat.their_public_key_input = key;
                    }
                    self.ui.chat.keys_status_message = Some(if is_active {
                        "✅ Ключи активны — сообщения шифруются".to_string()
                    } else {
                        "⚠️ Ключ собеседника не установлен — сообщения не зашифрованы".to_string()
                    });
                }
            }

            AppEvent::TheirPublicKeySet { conv_id } => {
                if self.ui.selected_conv_id == Some(conv_id) {
                    self.ui.chat.keys_status_message =
                        Some("✅ Ключ собеседника сохранён — сообщения шифруются".to_string());
                    self.ui.toast_info("Ключ собеседника сохранён");
                }
                // Перезагружаем ключи
                self.spawn_load_conversation_keys(conv_id);
            }

            AppEvent::KeysError(e) => {
                self.ui.toast_error(format!("Ошибка ключей: {}", e));
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
    /// Пробует автоматически войти с последним сохранённым аккаунтом.
    fn try_auto_login(&mut self) {
        let sender = self.rt.event_sender();
        let db_path = app_db_path();
        let config = AppConfig::default();

        // Устанавливаем флаг что идёт авто-вход
        self.ui.login.is_auto_login = true;

        self.rt.spawn(async move {
            match platform::restore_last_session(&db_path, config).await {
                Ok(Some((state, account))) => {
                    tracing::info!("Автоматический вход в аккаунт {}", account.email);
                    sender.send(AppEvent::AccountReady { state, account });
                }
                Ok(None) => {
                    tracing::info!("Нет сохранённых аккаунтов, показываем экран входа");
                    // Остаёмся на экране Login, сбрасываем флаг
                    sender.send(AppEvent::AutoLoginComplete);
                }
                Err(e) => {
                    tracing::warn!("Ошибка восстановления сессии: {}", e);
                    // При ошибке авто-входа просто переходим к обычному входу
                    sender.send(AppEvent::AutoLoginComplete);
                }
            }
        });
    }

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
                if let Some(conv_id) = sidebar::show(ui, &mut self.ui, &sender) {
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
                            let action = compose::show(ui, &mut self.ui.chat.draft, false);
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
                crate::state::ToastKind::Info => (theme::BG_MSG_IN, theme::TEXT_PRIMARY),
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
            let state = match platform::build_app_state(&email, &password, &db_path, config).await {
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
                        "Поддерживаются только Gmail, mail.ru и yandex.ru".into(),
                    ));
                    return;
                }
            };

            // Добавляем аккаунт (или получаем существующий)
            let account = match state
                .account_service
                .add_account(email.clone(), password.clone(), provider)
                .await
            {
                Ok(acc) => acc,
                // Уже существует — загружаем и убеждаемся что credentials есть
                Err(echat_core::Error::AlreadyExists(_)) => {
                    match state.account_service.list_accounts().await {
                        Ok(list) if !list.is_empty() => {
                            let acc = match list.into_iter().find(|a| a.email == email) {
                                Some(acc) => acc,
                                None => {
                                    // Этого не должно произойти, если мы получили AlreadyExists
                                    sender.send(AppEvent::AccountError(
                                        "Не удалось найти существующий аккаунт".into(),
                                    ));
                                    return;
                                }
                            };
                            // Сохраняем пароль в keystore (перезаписываем)
                            if let Err(e) = state
                                .account_service
                                .save_app_password(&email, &password)
                                .await
                            {
                                tracing::warn!("Не удалось сохранить пароль в keystore: {}", e);
                                // Не считаем это критичной ошибкой — продолжаем
                            }
                            
                            // Генерируем keypair если отсутствует
                            if let Err(e) =
                                state.account_service.load_or_create_keypair(acc.id).await
                            {
                                sender.send(AppEvent::AccountError(format!(
                                    "Не удалось создать identity ключ: {}",
                                    e
                                )));
                                return;
                            }
                            acc
                        }
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
        self.load_conversation_keys(conv_id);
    }

    fn load_history(&mut self, conv_id: Uuid) {
        let (state, _) = match self.services() {
            Some(x) => x,
            None => return,
        };
        let sender = self.rt.event_sender();

        self.rt.spawn(async move {
            match state.chat_service.get_history(conv_id, None, 60).await {
                Ok(msgs) => sender.send(AppEvent::HistoryLoaded {
                    conv_id,
                    messages: msgs,
                }),
                Err(e) => tracing::warn!("Ошибка загрузки истории: {}", e),
            }
        });
    }

    /// Обновить сообщения в текущем открытом диалоге из БД.
    /// В отличие от `load_history` не ставит флаги загрузки и не вызывает
    /// лишних перерисовок — просто тихо подтягивает актуальные данные.
    fn refresh_current_chat(&mut self, conv_id: Uuid) {
        // Не обновляем если пользователь ушёл с экрана Main или выбрал другой диалог
        if self.ui.selected_conv_id != Some(conv_id) {
            return;
        }
        let (state, _) = match self.services() {
            Some(x) => x,
            None => return,
        };
        let sender = self.rt.event_sender();

        // Запоминаем текущие ID чтобы избежать дубликатов
        let existing_ids: Vec<Uuid> = self.ui.chat.messages.iter().map(|m| m.id).collect();

        self.rt.spawn(async move {
            match state.chat_service.get_history(conv_id, None, 200).await {
                Ok(msgs) => {
                    // Фильтруем только новые сообщения
                    let new_msgs: Vec<_> = msgs
                        .into_iter()
                        .filter(|m| !existing_ids.contains(&m.id))
                        .collect();
                    if !new_msgs.is_empty() {
                        sender.send(AppEvent::HistoryLoaded {
                            conv_id,
                            messages: new_msgs,
                        });
                    }
                }
                Err(e) => tracing::warn!("Ошибка обновления истории: {}", e),
            }
        });
    }

    fn load_conversation_keys(&mut self, conv_id: Uuid) {
        let (state, _) = match self.services() {
            Some(x) => x,
            None => return,
        };
        let sender = self.rt.event_sender();

        self.rt.spawn(async move {
            match state.chat_service.get_conversation_keys(conv_id).await {
                Ok(keys) => {
                    sender.send(AppEvent::ConversationKeysLoaded {
                        conv_id,
                        my_public_key: keys.my_keypair_json.unwrap_or_default(),
                        their_public_key: keys.their_public_key_json,
                        is_active: keys.is_active,
                    });
                }
                Err(_) => {
                    // Ключей ещё нет — они будут созданы при первом открытии
                }
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
                Ok(msg) => sender.send(AppEvent::MessageSent {
                    conv_id,
                    message: msg,
                }),
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

    fn spawn_delete_contact(&mut self, contact_id: Uuid) {
        let (state, _) = match self.services() {
            Some(x) => x,
            None => return,
        };
        let sender = self.rt.event_sender();

        self.rt.spawn(async move {
            match state.contact_service.delete_contact(contact_id).await {
                Ok(_) => sender.send(AppEvent::ContactDeleted { contact_id }),
                Err(e) => sender.send(AppEvent::ContactError(e.to_string())),
            }
        });
    }

    fn open_chat_with(&mut self, contact_id: Uuid) {
        // Ищем уже существующую direct-беседу
        let existing_conv_id = self
            .ui
            .contacts
            .iter()
            .find(|c| c.id == contact_id)
            .and_then(|c| {
                self.ui
                    .conversations
                    .iter()
                    .find(|conv| {
                        // Используем display_name как прокси (до полного Conversation хранения)
                        conv.display_name == c.name || conv.display_name == c.email
                    })
                    .map(|conv| conv.id)
            });

        if let Some(conv_id) = existing_conv_id {
            self.select_conversation(conv_id);
            self.ui.screen = Screen::Main;
        } else {
            // Беседы нет — создаём новую
            self.spawn_create_chat(contact_id);
        }
    }

    fn spawn_create_chat(&mut self, contact_id: Uuid) {
        let (state, account_id) = match self.services() {
            Some(x) => x,
            None => return,
        };
        let sender = self.rt.event_sender();

        self.rt.spawn(async move {
            // Создаём беседу через ChatService
            let result = state
                .chat_service
                .get_or_create_direct_conversation(account_id, contact_id)
                .await;

            match result {
                Ok(conv) => {
                    // Загружаем обновлённый список бесед
                    match state.chat_service.list_conversations(account_id).await {
                        Ok(convs) => sender.send(AppEvent::ConversationsLoaded(convs)),
                        Err(e) => tracing::warn!("Ошибка загрузки бесед после создания: {}", e),
                    }
                    // Отправляем событие для открытия чата
                    sender.send(AppEvent::ChatCreated {
                        conv_id: conv.id,
                        contact_id,
                    });
                }
                Err(e) => {
                    tracing::warn!("Ошибка создания беседы: {}", e);
                }
            }
        });
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

    fn spawn_load_conversation_keys(&mut self, conv_id: Uuid) {
        let (state, account_id) = match self.services() {
            Some(x) => x,
            None => return,
        };
        let sender = self.rt.event_sender();

        self.rt.spawn(async move {
            match state.chat_service.get_conversation_keys(conv_id).await {
                Ok(keys) => {
                    // Извлекаем публичный ключ из my_keypair_json (seed в base64)
                    let my_public_key = keys.my_keypair_json.and_then(|seed_base64| {
                        // Декодируем seed и генерируем keypair
                        let seed_bytes = match base64::Engine::decode(
                            &base64::engine::general_purpose::STANDARD,
                            &seed_base64,
                        ) {
                            Ok(b) => b,
                            Err(_) => return None,
                        };
                        let seed_array: [u8; 32] = seed_bytes.try_into().ok()?;
                        let keypair = encryption::keypair::IdentityKeypair::from_seed(
                            encryption::keypair::KeySeed::from_bytes(seed_array),
                        );
                        // Возвращаем публичные ключи в base64
                        keypair.public_keys().to_base64().ok()
                    });

                    sender.send(AppEvent::ConversationKeysLoaded {
                        conv_id,
                        my_public_key: my_public_key.unwrap_or_default(),
                        their_public_key: keys.their_public_key_json,
                        is_active: keys.is_active,
                    });
                }
                Err(e) => sender.send(AppEvent::KeysError(e.to_string())),
            }
        });
    }

    fn spawn_set_their_public_key(&mut self, conv_id: Uuid, public_key_json: String) {
        let (state, account_id) = match self.services() {
            Some(x) => x,
            None => return,
        };
        let sender = self.rt.event_sender();

        self.rt.spawn(async move {
            // Сохраняем ключ (в conversation_keys и в контакт)
            match state
                .chat_service
                .set_their_public_key(conv_id, public_key_json)
                .await
            {
                Ok(()) => {
                    sender.send(AppEvent::TheirPublicKeySet { conv_id });

                    // Перезагружаем контакты чтобы обновить статус
                    match state.contact_service.list_contacts(account_id).await {
                        Ok(contacts) => sender.send(AppEvent::ContactsLoaded(contacts)),
                        Err(e) => tracing::warn!("Ошибка загрузки контактов: {}", e),
                    }
                }
                Err(e) => sender.send(AppEvent::KeysError(e.to_string())),
            }
        });
    }

    fn spawn_logout(&mut self) {
        let (state, account_id) = match self.services() {
            Some(x) => x,
            None => return,
        };
        let sender = self.rt.event_sender();

        self.rt.spawn(async move {
            // Останавливаем SyncEngine (задача завершится когда cmd_tx будет dropped)
            // Удаляем credentials из keystore и аккаунт из БД
            if let Err(e) = state.account_service.delete_account(account_id).await {
                tracing::warn!("Ошибка удаления аккаунта при выходе: {}", e);
            }

            sender.send(AppEvent::LogoutComplete);
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

fn provider_from_email(email: &str) -> Option<echat_core::models::account::Provider> {
    use echat_core::models::account::Provider;
    let domain = email.split('@').nth(1)?;
    match domain {
        "gmail.com" | "googlemail.com" => Some(Provider::Gmail),
        "mail.ru" | "inbox.ru" | "list.ru" | "bk.ru" => Some(Provider::MailRu),
        "yandex.ru" | "ya.ru" | "yandex.com" | "yandex.kz" | "yandex.by" => Some(Provider::Yandex),
        _ => None,
    }
}
