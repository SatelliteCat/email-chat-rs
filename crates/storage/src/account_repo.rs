//! Репозиторий аккаунтов.
//!
//! Аккаунт = почтовый ящик пользователя (может быть несколько).
//! Пароль приложения НЕ хранится в БД — только в platform/keystore.

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    Error, Result,
    models::{AccountRow, NewAccount, now_iso},
};

#[derive(Clone)]
pub struct AccountRepo {
    pool: SqlitePool,
}

impl AccountRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Создаёт новый аккаунт.
    pub async fn create(&self, account: &NewAccount) -> Result<()> {
        let id = account.id.to_string();
        let provider = account.provider.as_str();
        let now = now_iso();
        let imap_port_i64 = account.imap_port as i64;
        let smtp_port_i64 = account.smtp_port as i64;

        sqlx::query!(
            r#"
            INSERT INTO accounts
                (id, email, provider, imap_host, imap_port,
                 smtp_host, smtp_port, echat_folder, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            id,
            account.email,
            provider,
            account.imap_host,
            imap_port_i64,
            account.smtp_host,
            smtp_port_i64,
            account.echat_folder,
            now,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint failed") {
                Error::Conflict(format!("Аккаунт {} уже существует", account.email))
            } else {
                Error::Sqlx(e)
            }
        })?;

        Ok(())
    }

    /// Возвращает аккаунт по ID.
    pub async fn get_by_id(&self, id: Uuid) -> Result<AccountRow> {
        let id_str = id.to_string();
        sqlx::query_as!(
            AccountRow,
            r#"SELECT id AS "id!", email AS "email!", provider AS "provider!", imap_host AS "imap_host!", imap_port, smtp_host AS "smtp_host!", smtp_port, echat_folder AS "echat_folder!", last_imap_uid, last_sync_at, is_active, created_at AS "created_at!" FROM accounts WHERE id = ?"#,
            id_str
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("Аккаунт {}", id)))
    }

    /// Возвращает аккаунт по email.
    pub async fn get_by_email(&self, email: &str) -> Result<AccountRow> {
        sqlx::query_as!(
            AccountRow,
            r#"SELECT id AS "id!", email AS "email!", provider AS "provider!", imap_host AS "imap_host!", imap_port, smtp_host AS "smtp_host!", smtp_port, echat_folder AS "echat_folder!", last_imap_uid, last_sync_at, is_active, created_at AS "created_at!" FROM accounts WHERE email = ?"#,
            email
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("Аккаунт {}", email)))
    }

    /// Возвращает все аккаунты.
    pub async fn list(&self) -> Result<Vec<AccountRow>> {
        Ok(
            sqlx::query_as!(
                AccountRow,
                r#"SELECT id AS "id!", email AS "email!", provider AS "provider!", imap_host AS "imap_host!", imap_port, smtp_host AS "smtp_host!", smtp_port, echat_folder AS "echat_folder!", last_imap_uid, last_sync_at, is_active, created_at AS "created_at!" FROM accounts ORDER BY created_at"#
            )
            .fetch_all(&self.pool)
            .await?,
        )
    }

    /// Обновляет последний полученный IMAP UID и время синхронизации.
    pub async fn update_sync_state(&self, id: Uuid, last_uid: u32) -> Result<()> {
        let id_str = id.to_string();
        let uid = last_uid as i64;
        let now = now_iso();

        sqlx::query!(
            "UPDATE accounts SET last_imap_uid = ?, last_sync_at = ? WHERE id = ?",
            uid,
            now,
            id_str,
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Удаляет аккаунт и все связанные данные (CASCADE).
    pub async fn delete(&self, id: Uuid) -> Result<()> {
        let id_str = id.to_string();
        let rows = sqlx::query!("DELETE FROM accounts WHERE id = ?", id_str)
            .execute(&self.pool)
            .await?
            .rows_affected();

        if rows == 0 {
            return Err(Error::NotFound(format!("Аккаунт {}", id)));
        }
        Ok(())
    }
}
