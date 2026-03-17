-- Добавляем поле error_message для хранения ошибок отправки

-- Добавляем колонку для хранения ошибки
ALTER TABLE messages ADD COLUMN error_message TEXT;

-- Обновляем комментарий к статусу чтобы включить 'failed'
-- 'queued' | 'sending' | 'sent' | 'delivered' | 'read' | 'failed'

-- Индекс для поиска сообщений с ошибками (для повторной отправки)
CREATE INDEX IF NOT EXISTS idx_messages_failed
    ON messages(status, sent_at ASC)
    WHERE status = 'failed';
