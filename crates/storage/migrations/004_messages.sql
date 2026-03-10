-- Сообщения чата (уже расшифрованные)

CREATE TABLE IF NOT EXISTS messages (
    id              TEXT PRIMARY KEY,         -- UUID (= msg_id из ChatEnvelope)
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    account_id      TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    from_email      TEXT NOT NULL,
    body            TEXT,                     -- NULL для системных сообщений
    kind            TEXT NOT NULL DEFAULT 'text',
                                              -- 'text' | 'handshake' | 'group_event'
    status          TEXT NOT NULL DEFAULT 'sent',
                                              -- 'queued' | 'sending' | 'sent'
                                              -- | 'delivered' | 'read'
    reply_to        TEXT REFERENCES messages(id) ON DELETE SET NULL,
    -- IMAP метаданные для удаления с сервера
    imap_uid        INTEGER,                  -- NULL для исходящих (они в Sent)
    imap_folder     TEXT,
    -- Временные метки
    sent_at         TEXT NOT NULL,            -- из ChatEnvelope (время отправки)
    received_at     TEXT,                     -- когда получили через IMAP
    created_at      TEXT NOT NULL             -- когда вставили в БД
);

-- Основной индекс для загрузки истории чата
CREATE INDEX IF NOT EXISTS idx_messages_conversation
    ON messages(conversation_id, sent_at ASC);

-- Индекс для удаления по IMAP UID
CREATE INDEX IF NOT EXISTS idx_messages_imap
    ON messages(conversation_id, imap_uid)
    WHERE imap_uid IS NOT NULL;

-- Индекс для поиска по отправителю (нужен при sync)
CREATE INDEX IF NOT EXISTS idx_messages_from
    ON messages(account_id, from_email, sent_at DESC);
