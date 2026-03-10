//! SyncEngine — основной цикл получения сообщений.
//!
//! ## Алгоритм
//!
//! ```text
//! loop {
//!     IMAP IDLE (29 мин) ──► пришло новое письмо?
//!         да  ──► fetch_new → process_incoming × N
//!         нет ──► переподключиться и продолжить
//!
//!     при любой ошибке:
//!         wait(reconnect_delay) → retry
//!         reconnect_delay экспоненциально растёт до 5 минут
//! }
//! ```

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use uuid::Uuid;

use crate::{
    events::{ChatEvent, EventBus},
    ports::email::DynEmailTransport,
    ports::storage::DynStorage,
    services::{account::AccountService, chat::ChatService, contacts::ContactService},
};

/// Команды управления SyncEngine из UI.
#[derive(Debug, Clone)]
pub enum SyncCommand {
    /// Немедленно опросить сервер
    FetchNow,
    /// Остановить engine
    Stop,
}

/// Запускает SyncEngine в фоновой tokio-задаче.
///
/// Возвращает sender для отправки команд и handle задачи.
pub fn start(
    account_id: Uuid,
    email: DynEmailTransport,
    storage: DynStorage,
    account_svc: AccountService,
    contact_svc: ContactService,
    chat_svc: ChatService,
    events: EventBus,
) -> (tokio::sync::mpsc::Sender<SyncCommand>, tokio::task::JoinHandle<()>) {
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(32);

    let handle = tokio::spawn(async move {
        run_sync_loop(
            account_id,
            email,
            storage,
            account_svc,
            contact_svc,
            chat_svc,
            events,
            cmd_rx,
        )
        .await;
    });

    (cmd_tx, handle)
}

async fn run_sync_loop(
    account_id: Uuid,
    email: DynEmailTransport,
    storage: DynStorage,
    account_svc: AccountService,
    contact_svc: ContactService,
    chat_svc: ChatService,
    events: EventBus,
    mut cmd_rx: tokio::sync::mpsc::Receiver<SyncCommand>,
) {
    let mut reconnect_delay = Duration::from_secs(5);
    const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(300); // 5 минут

    tracing::info!("SyncEngine запущен для аккаунта {}", account_id);
    events.emit(ChatEvent::SyncStateChanged { connected: false });

    // Убеждаемся что папка EChat существует
    if let Err(e) = email.ensure_echat_folder().await {
        tracing::warn!("Не удалось создать папку EChat: {}", e);
    }

    loop {
        // Получаем последний синхронизированный UID
        let since_uid = match storage.get_account(account_id).await {
            Ok(acc) => acc.last_imap_uid,
            Err(e) => {
                tracing::error!("Не удалось получить аккаунт: {}", e);
                tokio::time::sleep(reconnect_delay).await;
                continue;
            }
        };

        // Сначала получаем все пропущенные письма
        events.emit(ChatEvent::SyncStateChanged { connected: true });
        reconnect_delay = Duration::from_secs(5); // сбрасываем задержку при успехе

        match fetch_and_process(
            account_id,
            since_uid,
            &email,
            &storage,
            &account_svc,
            &contact_svc,
            &chat_svc,
        )
        .await
        {
            Ok(max_uid) => {
                if let Some(uid) = max_uid {
                    if let Err(e) = storage.update_account_sync_state(account_id, uid).await {
                        tracing::warn!("Не удалось обновить sync state: {}", e);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Ошибка при получении писем: {}", e);
                events.emit(ChatEvent::SyncError { message: e.to_string() });
                events.emit(ChatEvent::SyncStateChanged { connected: false });
                tokio::time::sleep(reconnect_delay).await;
                reconnect_delay = (reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
                continue;
            }
        }

        // IDLE ожидание новых писем
        tokio::select! {
            idle_result = email.idle_wait() => {
                match idle_result {
                    Ok(has_new) => {
                        if has_new {
                            tracing::debug!("IDLE: новые письма, fetching...");
                            // Следующая итерация цикла подхватит их
                        }
                        // таймаут IDLE — тоже OK, просто переподключаемся
                    }
                    Err(e) => {
                        tracing::warn!("IDLE ошибка: {}. Переподключение...", e);
                        events.emit(ChatEvent::SyncStateChanged { connected: false });
                        tokio::time::sleep(reconnect_delay).await;
                        reconnect_delay = (reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
                    }
                }
            }

            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(SyncCommand::FetchNow) => {
                        tracing::debug!("SyncCommand::FetchNow получен");
                        // продолжаем цикл — fetch будет на следующей итерации
                    }
                    Some(SyncCommand::Stop) | None => {
                        tracing::info!("SyncEngine остановлен для аккаунта {}", account_id);
                        events.emit(ChatEvent::SyncStateChanged { connected: false });
                        return;
                    }
                }
            }
        }
    }
}

/// Получает и обрабатывает новые письма.
/// Возвращает максимальный UID среди обработанных (для сохранения в sync state).
async fn fetch_and_process(
    account_id: Uuid,
    since_uid: Option<u32>,
    email: &DynEmailTransport,
    storage: &DynStorage,
    account_svc: &AccountService,
    contact_svc: &ContactService,
    chat_svc: &ChatService,
) -> crate::Result<Option<u32>> {
    let messages = email.fetch_new(since_uid).await?;

    if messages.is_empty() {
        return Ok(None);
    }

    tracing::info!("Получено {} новых писем", messages.len());

    let mut max_uid: Option<u32> = None;

    for msg in messages {
        let uid = msg.uid;

        if let Err(e) = super::processor::process_incoming(
            &msg,
            account_id,
            account_svc,
            contact_svc,
            chat_svc,
        )
        .await
        {
            tracing::warn!("Ошибка обработки письма uid={}: {}", uid, e);
            // Не прерываем — продолжаем остальные
        }

        max_uid = Some(max_uid.map_or(uid, |m: u32| m.max(uid)));
    }

    Ok(max_uid)
}
