ALTER TABLE git_repos ADD COLUMN IF NOT EXISTS write_token_ciphertext TEXT;
ALTER TABLE git_repos DROP COLUMN IF EXISTS write_token_secret;
ALTER TABLE git_repos DROP COLUMN IF EXISTS write_token_key;
