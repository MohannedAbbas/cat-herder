-- Repo credentials are now minted at run time by the JIT credential broker (GitHub
-- App) instead of stored per-repo. Drop the long-lived encrypted token columns.
ALTER TABLE git_repos DROP COLUMN IF EXISTS token_ciphertext;
ALTER TABLE git_repos DROP COLUMN IF EXISTS write_token_ciphertext;
