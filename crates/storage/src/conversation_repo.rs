//! Репозиторий бесед (direct и групповые чаты) и участников групп.

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    Error, Result,
    models::{
        ConversationRow, GroupMemberRow, NewDirectConversation, NewGroupConversation,
        NewGroupMember, now_iso,
    },
};

#[derive(Clone)]
pub struct ConversationRepo {
    pool: SqlitePool,
}

impl ConversationRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // ── Создание ─────────────────────────────────────────────────────────────

    /// Создаёт direct-беседу с одним контактом.
    pub async fn create_direct(&self, conv: &NewDirectConversation) -> Result<()> {
        let id = conv.id.to_string();
        let account_id = conv.account_id.to_string();
        let contact_id = conv.contact_id.to_string();
        let now = now_iso();

        sqlx::query!(
            r#"
            INSERT INTO conversations
                (id, account_id, kind, contact_id, created_at, updated_at)
            VALUES (?, ?, 'direct', ?, ?, ?)
            "#,
            id,
            account_id,
            contact_id,
            now,
            now,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint failed") {
                Error::Conflict("Беседа уже существует".into())
            } else {
                Error::Sqlx(e)
            }
        })?;

        Ok(())
    }

    /// Создаёт групповой чат с первоначальными участниками.
    /// Выполняется в транзакции — либо всё, либо ничего.
    pub async fn create_group(&self, conv: &NewGroupConversation) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        let id = conv.id.to_string();
        let account_id = conv.account_id.to_string();
        let now = now_iso();

        sqlx::query!(
            r#"
            INSERT INTO conversations
                (id, account_id, kind, group_name, group_avatar, created_at, updated_at)
            VALUES (?, ?, 'group', ?, ?, ?, ?)
            "#,
            id,
            account_id,
            conv.name,
            conv.avatar,
            now,
            now,
        )
        .execute(&mut *tx)
        .await?;

        // Добавляем всех участников
        for member in &conv.members {
            let contact_id = member.contact_id.to_string();
            let role = member.role.as_str();

            sqlx::query!(
                r#"
                INSERT INTO group_members
                    (conversation_id, contact_id, role, joined_at, public_key_snapshot)
                VALUES (?, ?, ?, ?, ?)
                "#,
                id,
                contact_id,
                role,
                now,
                member.public_key_snapshot,
            )
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    // ── Чтение ───────────────────────────────────────────────────────────────

    /// Возвращает беседу по ID.
    pub async fn get_by_id(&self, id: Uuid) -> Result<ConversationRow> {
        let id_str = id.to_string();
        sqlx::query_as!(
            ConversationRow,
            r#"SELECT id AS "id!", account_id AS "account_id!", kind AS "kind!", contact_id, group_name, group_avatar, last_msg_at, last_msg_preview, unread_count, created_at AS "created_at!", updated_at AS "updated_at!" FROM conversations WHERE id = ?"#,
            id_str,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("Беседа {}", id)))
    }

    /// Возвращает direct-беседу с конкретным контактом.
    pub async fn find_direct(
        &self,
        account_id: Uuid,
        contact_id: Uuid,
    ) -> Result<Option<ConversationRow>> {
        let account_id_str = account_id.to_string();
        let contact_id_str = contact_id.to_string();

        Ok(sqlx::query_as!(
            ConversationRow,
            r#"SELECT id AS "id!", account_id AS "account_id!", kind AS "kind!", contact_id, group_name, group_avatar, last_msg_at, last_msg_preview, unread_count, created_at AS "created_at!", updated_at AS "updated_at!" FROM conversations WHERE account_id = ? AND kind = 'direct' AND contact_id = ?"#,
            account_id_str,
            contact_id_str,
        )
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Возвращает все беседы аккаунта, сортировка по последнему сообщению.
    pub async fn list(&self, account_id: Uuid) -> Result<Vec<ConversationRow>> {
        let account_id_str = account_id.to_string();
        Ok(sqlx::query_as!(
            ConversationRow,
            r#"SELECT id AS "id!", account_id AS "account_id!", kind AS "kind!", contact_id, group_name, group_avatar, last_msg_at, last_msg_preview, unread_count, created_at AS "created_at!", updated_at AS "updated_at!" FROM conversations WHERE account_id = ? ORDER BY COALESCE(last_msg_at, created_at) DESC"#,
            account_id_str,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// Возвращает участников группового чата.
    pub async fn get_members(&self, conv_id: Uuid) -> Result<Vec<GroupMemberRow>> {
        let id_str = conv_id.to_string();
        Ok(sqlx::query_as!(
            GroupMemberRow,
            r#"SELECT conversation_id AS "conversation_id!", contact_id AS "contact_id!", role AS "role!", joined_at AS "joined_at!", public_key_snapshot FROM group_members WHERE conversation_id = ?"#,
            id_str,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    // ── Обновление ───────────────────────────────────────────────────────────

    /// Обновляет превью последнего сообщения и счётчик непрочитанных.
    pub async fn update_last_message(
        &self,
        conv_id: Uuid,
        preview: &str,
        sent_at: &str,
        increment_unread: bool,
    ) -> Result<()> {
        let id_str = conv_id.to_string();
        let now = now_iso();
        let preview_truncated = &preview[..preview.len().min(60)];

        if increment_unread {
            sqlx::query!(
                r#"UPDATE conversations
                   SET last_msg_at = ?, last_msg_preview = ?,
                       unread_count = unread_count + 1, updated_at = ?
                   WHERE id = ?"#,
                sent_at,
                preview_truncated,
                now,
                id_str,
            )
            .execute(&self.pool)
            .await?;
        } else {
            sqlx::query!(
                r#"UPDATE conversations
                   SET last_msg_at = ?, last_msg_preview = ?, updated_at = ?
                   WHERE id = ?"#,
                sent_at,
                preview_truncated,
                now,
                id_str,
            )
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    /// Сбрасывает счётчик непрочитанных (пользователь открыл чат).
    pub async fn mark_as_read(&self, conv_id: Uuid) -> Result<()> {
        let id_str = conv_id.to_string();
        let now = now_iso();
        sqlx::query!(
            "UPDATE conversations SET unread_count = 0, updated_at = ? WHERE id = ?",
            now,
            id_str,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Добавляет участника в группу.
    pub async fn add_member(&self, conv_id: Uuid, member: &NewGroupMember) -> Result<()> {
        let conv_id_str = conv_id.to_string();
        let contact_id_str = member.contact_id.to_string();
        let role = member.role.as_str();
        let now = now_iso();

        sqlx::query!(
            r#"INSERT INTO group_members
                (conversation_id, contact_id, role, joined_at, public_key_snapshot)
               VALUES (?, ?, ?, ?, ?)"#,
            conv_id_str,
            contact_id_str,
            role,
            now,
            member.public_key_snapshot,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint failed") {
                Error::Conflict("Участник уже в группе".into())
            } else {
                Error::Sqlx(e)
            }
        })?;

        Ok(())
    }

    /// Удаляет участника из группы.
    pub async fn remove_member(&self, conv_id: Uuid, contact_id: Uuid) -> Result<()> {
        let conv_id_str = conv_id.to_string();
        let contact_id_str = contact_id.to_string();

        let rows = sqlx::query!(
            "DELETE FROM group_members WHERE conversation_id = ? AND contact_id = ?",
            conv_id_str,
            contact_id_str,
        )
        .execute(&self.pool)
        .await?
        .rows_affected();

        if rows == 0 {
            return Err(Error::NotFound("Участник не найден в группе".into()));
        }
        Ok(())
    }

    // ── Удаление ─────────────────────────────────────────────────────────────

    /// Удаляет беседу и все связанные данные (CASCADE).
    /// Сообщения и участники удаляются автоматически.
    pub async fn delete(&self, id: Uuid) -> Result<()> {
        let id_str = id.to_string();
        let rows = sqlx::query!("DELETE FROM conversations WHERE id = ?", id_str)
            .execute(&self.pool)
            .await?
            .rows_affected();

        if rows == 0 {
            return Err(Error::NotFound(format!("Беседа {}", id)));
        }
        Ok(())
    }
}
