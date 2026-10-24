ALTER TABLE git_repos ADD COLUMN IF NOT EXISTS write_token_secret TEXT;
ALTER TABLE git_repos ADD COLUMN IF NOT EXISTS write_token_key TEXT;
