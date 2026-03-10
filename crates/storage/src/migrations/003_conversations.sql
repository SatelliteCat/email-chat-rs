-- Беседы: direct и групповые чаты

CREATE TABLE IF NOT EXISTS conversations (
    id           TEXT PRIMARY KEY,           -- UUID (= conv_id в ChatEnvelope)
    account_id   TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL,              -- 'direct' | 'group'
    -- Для direct: contact_id собеседника
    contact_id   TEXT REFERENCES contacts(id) ON DELETE SET NULL,
    -- Для group: название и аватар
    group_name   TEXT,
    group_avatar BLOB,
    -- Последнее сообщение (для сортировки списка чатов)
    last_msg_at  TEXT,
    last_msg_preview TEXT,                   -- первые ~60 символов
    -- Счётчик непрочитанных
    unread_count INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_conversations_account
    ON conversations(account_id, last_msg_at DESC);

-- Участники групповых чатов

CREATE TABLE IF NOT EXISTS group_members (
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    contact_id      TEXT NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    role            TEXT NOT NULL DEFAULT 'member', -- 'owner' | 'admin' | 'member'
    joined_at       TEXT NOT NULL,
    -- Публичный ключ на момент добавления (снапшот для ротации ключей)
    public_key_snapshot TEXT,

    PRIMARY KEY (conversation_id, contact_id)
);
