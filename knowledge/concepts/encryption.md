---
type: Concept
title: Encryption key management
description: AES-256-GCM encryption of git tokens at rest, the key's lifecycle in a Kubernetes Secret, and online rotation.
tags: [security, encryption, aes-gcm, kubernetes, secret, key-rotation]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: crypto, resource: /control-plane/crates/adapters/api-http/src/crypto.rs, title: crypto.rs }
  - { id: keystore, resource: /control-plane/crates/adapters/api-http/src/key_store.rs, title: key_store.rs }
  - { id: rotate, resource: /control-plane/crates/adapters/api-http/src/admin.rs, title: rotate_encryption_key }
  - { id: rbac, resource: /helm/cat-herder/templates/rbac.yaml, title: rbac.yaml }
---

# Encryption key management

Git **read** and **write** tokens ([GitRepo](/concepts/git-repo.md)) are encrypted at
rest with **AES-256-GCM** (via `ring`). The key is kept **out of the database**, in a
Kubernetes Secret, and owned by the control plane.

## Cipher (`AesGcmCipher`)

- Key = base64-encoded **32 bytes** (`from_base64_key`).
- Ciphertext format: **`v1:<base64(nonce ‖ ciphertext ‖ tag)>`** — a fresh random nonce
  per encryption (so equal plaintexts produce different ciphertexts); the `v1:` prefix
  leaves room to change algorithms.
- `generate()` / `generate_key_base64()` create a random key.

## Key store (`KeySecretStore`)

- Targets a namespace + Secret name + key. Defaults: Secret `cat-herder-encryption`, key
  `key` (env `CAT_HERDER_ENCRYPTION_SECRET` / `CAT_HERDER_ENCRYPTION_SECRET_KEY`).
- `load_or_create()` on boot: reads the key if present/valid, else **generates and
  writes** one — no operator initialization required.
- Writes use **server-side apply** (`Patch::Apply`, field manager `cat-herder`) so the
  control plane needs only `patch`/`create` scoped to that one Secret name (see
  `rbac.yaml`).

**Boot is fail-closed**: the [binary](/crates/bin-cat-herder.md) `expect`s the key store
to connect and initialize; it will not start without it.

## Rotation (`POST /v1/admin/settings/encryption-key/rotate`)

Admin-only and audited (`encryption-key.rotate`). Ordered for crash safety:

1. Decrypt + re-encrypt every stored token **in memory** under a fresh key (nothing
   persisted yet; failure leaves the old key untouched).
2. Persist the **new key to the Secret first**.
3. Save the re-encrypted tokens.
4. Swap the active in-memory cipher.

## Related

- [GitRepo](/concepts/git-repo.md) · [PR/MR creation](/concepts/pr-creation.md)
- [Gotchas](/operations/gotchas.md) (never delete the encryption Secret)
