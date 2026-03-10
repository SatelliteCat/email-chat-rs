-- Аккаунты пользователя (почтовые ящики)
-- Пароль приложения НЕ хранится здесь — только через platform/keystore

CREATE TABLE IF NOT EXISTS accounts (
    id           TEXT PRIMARY KEY,           -- UUID
    email        TEXT NOT NULL UNIQUE,
    provider     TEXT NOT NULL,              -- 'mailru' | 'yandex'
    imap_host    TEXT NOT NULL,
    imap_port    INTEGER NOT NULL,
    smtp_host    TEXT NOT NULL,
    smtp_port    INTEGER NOT NULL,
    echat_folder TEXT NOT NULL DEFAULT 'EChat',
    -- Состояние синхронизации
    last_imap_uid INTEGER,                   -- последний полученный UID
    last_sync_at  TEXT,                      -- ISO 8601 UTC
    is_active     INTEGER NOT NULL DEFAULT 1, -- 0/1 (SQLite bool)
    created_at    TEXT NOT NULL
);
