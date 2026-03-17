-- Ключи диалогов (пара ключей на каждый direct-чат)

CREATE TABLE IF NOT EXISTS conversation_keys (
    conversation_id   TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
    -- Наш полный keypair для этого диалога (X25519 + Ed25519, JSON с приватным ключом)
    my_keypair_json   TEXT,
    -- Публичный ключ собеседника (X25519 + Ed25519, JSON)
    their_public_key_json TEXT,
    -- Статус ключей
    status            TEXT NOT NULL DEFAULT 'incomplete',
                      -- 'incomplete' | 'active'
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_conversation_keys_status
    ON conversation_keys(status);
