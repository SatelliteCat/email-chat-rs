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

use std::time::Duration;
use uuid::Uuid;

use crate::{
    events::{ChatEvent, EventBus},
    ports::email::DynEmailTransport,
    ports::storage::DynStorage,
    services::{account::AccountService, chat::ChatService},
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
    chat_svc: ChatService,
    events: EventBus,
) -> (
    tokio::sync::mpsc::Sender<SyncCommand>,
    tokio::task::JoinHandle<()>,
) {
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(32);

    let handle = tokio::spawn(async move {
        run_sync_loop(
            account_id,
            email,
            storage,
            account_svc,
            chat_svc,
            events,
            cmd_rx,
        )
        .await;
    });

    (cmd_tx, handle)
}

pub(crate) async fn run_sync_loop(
    account_id: Uuid,
    email: DynEmailTransport,
    storage: DynStorage,
    account_svc: AccountService,
    chat_svc: ChatService,
    events: EventBus,
    mut cmd_rx: tokio::sync::mpsc::Receiver<SyncCommand>,
) {
    let mut reconnect_delay = Duration::from_secs(5);
    const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30); // 0,5 минут
    
    // Heartbeat — проверка соединения каждые 60 секунд
    let mut heartbeat_interval = tokio::time::interval(Duration::from_secs(60));

    tracing::info!("SyncEngine запущен для аккаунта {}", account_id);
    events.emit(ChatEvent::SyncStateChanged { connected: false });

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

        tracing::info!("Sync цикл: last_imap_uid={:?}", since_uid);

        // Сначала получаем все пропущенные письма
        events.emit(ChatEvent::SyncStateChanged { connected: true });
        reconnect_delay = Duration::from_secs(5); // сбрасываем задержку при успехе

        match fetch_and_process(
            account_id,
            since_uid,
            &email,
            &account_svc,
            &chat_svc,
        )
        .await
        {
            Ok(max_uid) => {
                if let Some(uid) = max_uid {
                    tracing::info!("Обновляю last_imap_uid={} для аккаунта {}", uid, account_id);
                    if let Err(e) = storage.update_account_sync_state(account_id, uid).await {
                        tracing::warn!("Не удалось обновить sync state: {}", e);
                    }
                } else {
                    tracing::debug!("Новых писем нет, last_imap_uid не обновляется");
                }
            }
            Err(e) => {
                tracing::warn!("Ошибка при получении писем: {}", e);
                events.emit(ChatEvent::SyncError {
                    message: e.to_string(),
                });
                events.emit(ChatEvent::SyncStateChanged { connected: false });
                tokio::time::sleep(reconnect_delay).await;
                reconnect_delay = (reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
                continue;
            }
        }

        // IDLE ожидание новых писем
        tracing::debug!("Запуск IMAP IDLE для ожидания новых писем...");
        tokio::select! {
            idle_result = email.idle_wait() => {
                match idle_result {
                    Ok(has_new) => {
                        tracing::debug!("IMAP IDLE: has_new={}", has_new);
                        if has_new {
                            tracing::info!("Получено уведомление о новых письмах через IDLE");
                            // Следующая итерация цикла подхватит их
                        } else {
                            tracing::debug!("IDLE таймаут (29 мин) — переподключение");
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

            // Heartbeat — проверка соединения каждые 60 секунд
            _ = heartbeat_interval.tick() => {
                tracing::debug!("Heartbeat: проверка соединения...");
                // Просто продолжаем цикл — fetch проверит соединение
            }

            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(SyncCommand::FetchNow) => {
                        tracing::info!("SyncCommand::FetchNow получен — принудительная синхронизация");
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
    account_svc: &AccountService,
    chat_svc: &ChatService,
) -> crate::Result<Option<u32>> {
    tracing::info!("Fetch новых писем начиная с UID: {:?}", since_uid);

    let messages = email.fetch_new(since_uid).await?;

    tracing::info!("Получено {} писем из IMAP", messages.len());

    if messages.is_empty() {
        tracing::debug!("Новых писем нет");
        return Ok(None);
    }

    let mut max_uid: Option<u32> = None;

    for msg in messages {
        let uid = msg.uid;

        match super::processor::process_incoming(
            &msg,
            account_id,
            email,
            account_svc,
            chat_svc,
        )
        .await
        {
            Ok(()) => {
                tracing::debug!("Письмо uid={} успешно обработано", uid);
            }
            Err(e) => {
                // Игнорируем дедупликацию — письмо уже обработано
                if e.to_string().contains("Уже существует") {
                    tracing::debug!("Письмо uid={} уже обработано (дедупликация)", uid);
                } else {
                    tracing::warn!("Ошибка обработки письма uid={}: {}", uid, e);
                }
            }
        }

        max_uid = Some(max_uid.map_or(uid, |m: u32| m.max(uid)));
    }

    Ok(max_uid)
}
