-- The public verification key is an application setting, not a secret vault.
ALTER TABLE applications ADD COLUMN tauri_public_key TEXT
    CHECK (tauri_public_key IS NULL OR (length(tauri_public_key) BETWEEN 1 AND 4096));
