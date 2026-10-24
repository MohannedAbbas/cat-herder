//! Storage for the encryption key in a Kubernetes Secret.
//!
//! The control plane owns this Secret: it reads the key on boot, generates a fresh
//! key and writes it if none exists, and rewrites it on rotation. This keeps the key
//! out of the database (separated from the ciphertext) and under k8s RBAC.
//!
//! Writes use server-side apply (the `patch` verb) so the control plane only needs
//! `patch` access scoped to this single Secret — not cluster-wide `create`/`update`.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use k8s_openapi::api::core::v1::Secret;
use kube::api::{Api, Patch, PatchParams};
use kube::Client;

use cat_herder_domain::ports::DomainResult;

use crate::crypto::AesGcmCipher;

const FIELD_MANAGER: &str = "cat-herder";

/// Reads/writes a single-key Secret holding the encryption key (base64-encoded).
#[derive(Clone)]
pub struct KeySecretStore {
    secrets: Api<Secret>,
    namespace: String,
    name: String,
    key: String,
}

impl KeySecretStore {
    /// Connect to the cluster and target `namespace`/`name`/`key`.
    pub async fn connect(namespace: &str, name: &str, key: &str) -> DomainResult<Self> {
        let client = Client::try_default().await?;
        Ok(Self {
            secrets: Api::namespaced(client, namespace),
            namespace: namespace.to_string(),
            name: name.to_string(),
            key: key.to_string(),
        })
    }

    /// Read the key from the Secret, or `None` if the Secret/key is absent.
    pub async fn load(&self) -> DomainResult<Option<String>> {
        match self.secrets.get(&self.name).await {
            Ok(secret) => {
                let value = secret
                    .data
                    .and_then(|d| d.get(&self.key).cloned())
                    .map(|bytes| String::from_utf8_lossy(&bytes.0).into_owned());
                Ok(value)
            }
            Err(kube::Error::Api(err)) if err.code == 404 => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Read an arbitrary key from an arbitrary Secret in this namespace (used to
    /// fetch a repo's write token). Returns `None` if the Secret/key is absent.
    pub async fn read(&self, name: &str, key: &str) -> DomainResult<Option<String>> {
        match self.secrets.get(name).await {
            Ok(secret) => {
                let value = secret
                    .data
                    .and_then(|d| d.get(key).cloned())
                    .map(|bytes| String::from_utf8_lossy(&bytes.0).into_owned());
                Ok(value)
            }
            Err(kube::Error::Api(err)) if err.code == 404 => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Write the key to the Secret (create-or-update via server-side apply).
    pub async fn store(&self, value: &str) -> DomainResult<()> {
        let mut data = serde_json::Map::new();
        data.insert(self.key.clone(), serde_json::Value::String(STANDARD.encode(value.as_bytes())));

        let apply = serde_json::json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": {
                "name": self.name,
                "namespace": self.namespace,
            },
            "type": "Opaque",
            "data": data,
        });
        self.secrets
            .patch(&self.name, &PatchParams::apply(FIELD_MANAGER).force(), &Patch::Apply(apply))
            .await?;
        Ok(())
    }

    /// Load the existing key, or generate + store a fresh one on first boot.
    pub async fn load_or_create(&self) -> DomainResult<String> {
        if let Some(key) = self.load().await?
            && AesGcmCipher::from_base64_key(&key).is_ok()
        {
            return Ok(key);
        }
        let key = AesGcmCipher::generate_key_base64()?;
        self.store(&key).await?;
        Ok(key)
    }
}
