//! Репозиторий сообщений.
//!
//! Ключевая особенность: хранит IMAP UID каждого входящего письма,
//! что позволяет удалять их с сервера при удалении чата.

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    Error, Result,
    models::{ImapUidRecord, MessageRow, MessageStatus, NewMessage, now_iso},
};

#[derive(Clone)]
pub struct MessageRepo {
    pool: SqlitePool,
}

impl MessageRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // ── Создание ─────────────────────────────────────────────────────────────

    /// Сохраняет новое сообщение.
    pub async fn create(&self, msg: &NewMessage) -> Result<()> {
        let id = msg.id.to_string();
        let conv_id = msg.conversation_id.to_string();
        let account_id = msg.account_id.to_string();
        let kind = msg.kind.as_str();
        let status = msg.status.as_str();
        let imap_uid = msg.imap_uid.map(|u| u as i64);
        let sent_at = msg.sent_at.to_rfc3339();
        let now = now_iso();

        let reply_to_id = msg.reply_to.map(|id| id.to_string());
        let reply_to_account_id = msg.reply_to_account_id.map(|id| id.to_string());

        sqlx::query(
            r#"
            INSERT INTO messages
                (id, conversation_id, account_id, from_email, body, kind,
                 status, reply_to_id, reply_to_account_id, imap_uid, imap_folder, sent_at, error_message, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(id)
        .bind(conv_id)
        .bind(account_id)
        .bind(&msg.from_email)
        .bind(&msg.body)
        .bind(&kind)
        .bind(&status)
        .bind(&reply_to_id)
        .bind(&reply_to_account_id)
        .bind(imap_uid)
        .bind(&msg.imap_folder)
        .bind(&sent_at)
        .bind(&msg.error_message)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint failed") {
                Error::Conflict(format!("Сообщение {} уже существует", msg.id))
            } else {
                Error::Sqlx(e)
            }
        })?;

        Ok(())
    }

    // ── Чтение ───────────────────────────────────────────────────────────────

    /// Возвращает историю беседы (пагинация через cursor).
    ///
    /// Загружает `limit` сообщений, начиная с `before_sent_at` (не включительно).
    /// Для первой загрузки `before_sent_at` = None.
    pub async fn get_history(
        &self,
        conv_id: Uuid,
        before_sent_at: Option<&str>,
        limit: i64,
    ) -> Result<Vec<MessageRow>> {
        let id_str = conv_id.to_string();

        let rows = if let Some(before) = before_sent_at {
            sqlx::query_as(
                r#"SELECT id, conversation_id, account_id, from_email, body, kind, status, reply_to_id, imap_uid, imap_folder, sent_at, received_at, error_message, created_at
                   FROM messages
                   WHERE conversation_id = ? AND sent_at < ?
                   ORDER BY sent_at DESC
                   LIMIT ?"#,
            )
            .bind(&id_str)
            .bind(before)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as(
                r#"SELECT id, conversation_id, account_id, from_email, body, kind, status, reply_to_id, imap_uid, imap_folder, sent_at, received_at, error_message, created_at
                   FROM messages
                   WHERE conversation_id = ?
                   ORDER BY sent_at DESC
                   LIMIT ?"#,
            )
            .bind(&id_str)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };

        // Разворачиваем обратно (DESC → ASC для отображения)
        let mut rows = rows;
        rows.reverse();
        Ok(rows)
    }

    /// Проверяет существование сообщения по ID (для дедупликации).
    /// Проверяем по account_id + msg_id чтобы разные аккаунты могли иметь сообщения с одинаковым msg_id.
    pub async fn exists(&self, id: Uuid, account_id: Option<Uuid>) -> Result<bool> {
        let id_str = id.to_string();
        let count: i64 = if let Some(acc_id) = account_id {
            let acc_id_str = acc_id.to_string();
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM messages WHERE id = ? AND account_id = ?",
            )
            .bind(&id_str)
            .bind(&acc_id_str)
            .fetch_one(&self.pool)
            .await?
        } else {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM messages WHERE id = ?",
            )
            .bind(&id_str)
            .fetch_one(&self.pool)
            .await?
        };
        Ok(count > 0)
    }

    // ── Обновление статуса ───────────────────────────────────────────────────

    /// Обновляет статус сообщения.
    pub async fn update_status(&self, id: Uuid, status: MessageStatus) -> Result<()> {
        let id_str = id.to_string();
        let status_str = status.as_str();
        sqlx::query!(
            "UPDATE messages SET status = ? WHERE id = ?",
            status_str,
            id_str,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Обновляет статус сообщения и сохраняет ошибку (если есть).
    pub async fn update_status_with_error(
        &self,
        id: Uuid,
        status: MessageStatus,
        error: Option<&str>,
    ) -> Result<()> {
        let id_str = id.to_string();
        let status_str = status.as_str();
        sqlx::query!(
            "UPDATE messages SET status = ?, error_message = ? WHERE id = ?",
            status_str,
            error,
            id_str,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Сохраняет IMAP UID после успешной отправки (письмо появилось в Sent).
    pub async fn set_imap_uid(&self, id: Uuid, uid: u32, folder: &str) -> Result<()> {
        let id_str = id.to_string();
        let uid_i64 = uid as i64;
        sqlx::query!(
            "UPDATE messages SET imap_uid = ?, imap_folder = ? WHERE id = ?",
            uid_i64,
            folder,
            id_str,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Удаление ─────────────────────────────────────────────────────────────

    /// Возвращает все IMAP UID сообщений беседы для удаления с сервера.
    ///
    /// Группирует по папкам (могут быть в EChat и в Sent).
    pub async fn get_imap_uids_for_deletion(&self, conv_id: Uuid) -> Result<Vec<ImapUidRecord>> {
        let id_str = conv_id.to_string();
        Ok(sqlx::query_as!(
            ImapUidRecord,
            r#"SELECT imap_uid, imap_folder
               FROM messages
               WHERE conversation_id = ?
                 AND imap_uid IS NOT NULL
                 AND imap_folder IS NOT NULL"#,
            id_str,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// Удаляет все сообщения беседы из БД.
    /// Вызывается ПОСЛЕ удаления писем с IMAP сервера.
    pub async fn delete_conversation_messages(&self, conv_id: Uuid) -> Result<u64> {
        let id_str = conv_id.to_string();
        let rows = sqlx::query!("DELETE FROM messages WHERE conversation_id = ?", id_str,)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(rows)
    }

    // ── Очередь на отправку ──────────────────────────────────────────────────

    /// Возвращает сообщения в статусе `queued` (контакт без приложения).
    pub async fn get_queued(&self, account_id: Uuid) -> Result<Vec<MessageRow>> {
        let id_str = account_id.to_string();
        Ok(sqlx::query_as(
            r#"SELECT id, conversation_id, account_id, from_email, body, kind, status, reply_to_id, imap_uid, imap_folder, sent_at, received_at, error_message, created_at
               FROM messages
               WHERE account_id = ? AND status = 'queued'
               ORDER BY sent_at ASC"#,
        )
        .bind(&id_str)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Возвращает сообщения в статусе `failed` для повторной отправки.
    pub async fn get_failed(&self, account_id: Uuid) -> Result<Vec<MessageRow>> {
        let id_str = account_id.to_string();
        Ok(sqlx::query_as(
            r#"SELECT id, conversation_id, account_id, from_email, body, kind, status, reply_to_id, imap_uid, imap_folder, sent_at, received_at, error_message, created_at
               FROM messages
               WHERE account_id = ? AND status = 'failed'
               ORDER BY sent_at ASC"#,
        )
        .bind(&id_str)
        .fetch_all(&self.pool)
        .await?)
    }
}
