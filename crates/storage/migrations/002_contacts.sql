-- Контакты пользователя

CREATE TABLE IF NOT EXISTS contacts (
    id          TEXT PRIMARY KEY,            -- UUID
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    email       TEXT NOT NULL,
    avatar      BLOB,                        -- PNG/JPEG байты, может быть NULL
    -- Состояние E2E канала
    status      TEXT NOT NULL DEFAULT 'unregistered',
                                             -- 'unregistered' | 'pending' | 'active'
    -- Публичные ключи (X25519 + Ed25519, JSON-сериализованные)
    -- NULL пока не завершён handshake
    public_keys_json TEXT,
    -- Дата последнего handshake
    handshake_at TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,

    UNIQUE(account_id, email)
);

CREATE INDEX IF NOT EXISTS idx_contacts_account
    ON contacts(account_id);

CREATE INDEX IF NOT EXISTS idx_contacts_email
    ON contacts(account_id, email);
