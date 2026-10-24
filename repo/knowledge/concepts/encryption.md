---
type: Concept
title: Encryption key management
description: AES-256-GCM encryption of git tokens, with the key generated into and rotated through a Kubernetes Secret.
tags: [security, encryption, aes-gcm, secret, rotation]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: crypto, resource: /crates/adapters/api-http/src/crypto.rs, title: AesGcmCipher }
  - { id: keystore, resource: /crates/adapters/api-http/src/key_store.rs, title: KeySecretStore }
  - { id: admin, resource: /crates/adapters/api-http/src/admin.rs, title: rotate_encryption_key }
  - { id: rbac, resource: /helm/cat-herder/templates/rbac.yaml, title: RBAC for the Secret }
---

# Encryption key management

Git tokens ([GitRepo](/concepts/git-repo.md)) are encrypted at rest with
**AES-256-GCM** (RustCrypto `ring`). The ciphertext lives in Postgres; the key lives
**separately** in a Kubernetes Secret.

## Cipher (`crates/adapters/api-http/src/crypto.rs`)

- Key: a base64-encoded 32-byte value; `AesGcmCipher::from_base64_key` validates the
  length; `generate_key_base64` produces a fresh random key.
- `encrypt(plaintext)` → `v1:<base64(nonce ‖ ciphertext ‖ tag)>` with a fresh random
  nonce (so encryption is non-deterministic).
- `decrypt` requires the `v1:` version prefix and rejects wrong versions/short input.
- The `v1:` prefix leaves room to rotate algorithms later.

## Key store (`crates/adapters/api-http/src/key_store.rs`)

`KeySecretStore` reads and writes a single-key Secret in the control plane's namespace
(default Secret `cat-herder-encryption`, key `key`; configured via
`CAT_HERDER_ENCRYPTION_SECRET` / `_KEY`). It:

- `load()` returns `None` on 404;
- `store()` writes via **server-side apply** (so only `patch`/`create` on that one
  Secret is needed);
- `load_or_create()` loads the existing key or generates + stores a fresh one on first
  boot. No operator initialization is required.

The [binary](/crates/bin-cat-herder.md) calls `load_or_create` on startup and
**fails loudly** if the key store is unavailable.

## Rotation

`POST /v1/admin/settings/encryption-key/rotate` (admin-only) does a careful multi-phase
rotation:

1. decrypt + re-encrypt every stored repo token **in memory** under a new key (nothing
   persisted yet; any failure aborts with the old key intact);
2. persist the new key to the Secret **first**;
3. save the re-encrypted tokens under the new key;
4. swap the in-memory `AesGcmCipher`;
5. write an audit entry.

## RBAC

The chart Role grants `get`/`list` on `secrets` for reading provider API-key Secrets,
and `get`/`patch`/`create` scoped by `resourceNames` to just the encryption-key
Secret for the control plane to own it ([Helm chart](/deploy/helm-chart.md)).

## Why separate key and ciphertext

Keeping the key in a Secret (under k8s RBAC, out of the database) means a database
dump alone does not expose tokens. See [Gotchas](/operations/gotchas.md).

## Related

- [GitRepo](/concepts/git-repo.md) · [PR/MR creation](/concepts/pr-creation.md) · [SSO](/concepts/sso.md)
