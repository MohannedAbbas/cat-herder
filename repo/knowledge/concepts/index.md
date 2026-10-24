# Concepts

- [Run](/concepts/run.md) — the unit of execution and its fields.
- [Run lifecycle](/concepts/run-lifecycle.md) — state machine and the end-to-end trace of one run.
- [Sandbox](/concepts/sandbox.md) — the agent-sandbox CRD pod per run.
- [GitRepo](/concepts/git-repo.md) — host/path identity, read vs write tokens.
- [Provider / Model / Agent catalog](/concepts/catalog.md) — LLM providers, models, agent templates, tools.
- [Encryption key management](/concepts/encryption.md) — AES-256-GCM and the k8s Secret.
- [Egress denial](/concepts/egress-policy.md) — default-deny NetworkPolicy for sandbox pods.
- [SSO (GitHub/GitLab OAuth)](/concepts/sso.md) — optional OAuth login.
- [PR/MR creation](/concepts/pr-creation.md) — opening PRs/MRs from the agent's diff.
