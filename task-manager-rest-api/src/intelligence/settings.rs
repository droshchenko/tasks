use super::provider::{EmbeddingConfig, env_value};
use crate::{app::AppContext, postgres::AppSettingDto};
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use task_manager_shared::ai_settings::{
    AiSettingsResponse, ProviderSettingsResponse, SaveProviderInput,
};

pub const JEV_ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";

pub struct Configuration {
    rows: RwLock<HashMap<String, AppSettingDto>>,
    cipher: Aes256Gcm,
    pub mutations: tokio::sync::Mutex<()>,
}

#[derive(Clone, Serialize, Deserialize)]
struct SealedKey {
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize)]
struct StoredProvider {
    enabled: bool,
    endpoint: String,
    model: String,
    key: Option<SealedKey>,
}

pub struct JevConfig {
    pub key: String,
    pub model: String,
}

impl Configuration {
    pub fn new(master: &str, rows: Vec<AppSettingDto>) -> Self {
        let key = Sha256::digest(format!("task-manager/provider-secrets/v1\0{master}").as_bytes());
        Self {
            rows: RwLock::new(rows.into_iter().map(|row| (row.id.clone(), row)).collect()),
            cipher: Aes256Gcm::new_from_slice(key.as_slice()).expect("SHA-256 key length"),
            mutations: tokio::sync::Mutex::new(()),
        }
    }

    pub fn get(&self, id: &str) -> Option<AppSettingDto> {
        self.rows.read().get(id).cloned()
    }
    pub fn install(&self, row: AppSettingDto) {
        self.rows.write().insert(row.id.clone(), row);
    }
    pub fn revision(&self, id: &str) -> i64 {
        self.rows.read().get(id).map_or(0, |row| row.revision)
    }

    fn seal(&self, provider: &str, key: &str) -> Result<SealedKey, String> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: key.as_bytes(),
                    aad: provider.as_bytes(),
                },
            )
            .map_err(|_| "provider key could not be encrypted")?;
        Ok(SealedKey {
            nonce: nonce.to_vec(),
            ciphertext,
        })
    }

    fn unseal(&self, provider: &str, key: &SealedKey) -> Result<String, String> {
        if key.nonce.len() != 12 {
            return Err("saved provider key is invalid; replace it in Settings".into());
        }
        let bytes = self
            .cipher
            .decrypt(
                Nonce::from_slice(&key.nonce),
                Payload {
                    msg: &key.ciphertext,
                    aad: provider.as_bytes(),
                },
            )
            .map_err(|_| "saved provider key cannot be decrypted; replace it in Settings")?;
        String::from_utf8(bytes)
            .map_err(|_| "saved provider key is invalid; replace it in Settings".into())
    }

    fn stored(&self, provider: &str) -> Result<Option<(StoredProvider, i64)>, String> {
        validate_provider(provider)?;
        self.get(&format!("provider:{provider}"))
            .map(|row| {
                serde_json::from_str(&row.value)
                    .map(|value| (value, row.revision))
                    .map_err(|_| "saved provider settings could not be read".to_string())
            })
            .transpose()
    }

    pub fn embeddings(&self) -> Result<Option<EmbeddingConfig>, String> {
        let Some((stored, _)) = self.stored("embeddings")? else {
            let config = EmbeddingConfig::from_env()?;
            if let Some(config) = &config {
                validate_endpoint(&config.endpoint)?;
                validate_model(&config.model)?;
            }
            return Ok(config);
        };
        if !stored.enabled {
            return Ok(None);
        }
        validate_endpoint(&stored.endpoint)?;
        validate_model(&stored.model)?;
        Ok(Some(EmbeddingConfig {
            endpoint: stored.endpoint,
            model: stored.model,
            key: stored
                .key
                .as_ref()
                .map(|key| self.unseal("embeddings", key))
                .transpose()?,
        }))
    }

    pub fn jev(&self) -> Result<Option<JevConfig>, String> {
        let Some((stored, _)) = self.stored("jev")? else {
            return Ok(env_value("TASKS_JEV_API_KEY").map(|key| JevConfig {
                key,
                model: env_value("TASKS_JEV_MODEL").unwrap_or_else(|| "jev-latest".into()),
            }));
        };
        if !stored.enabled {
            return Ok(None);
        }
        validate_model(&stored.model)?;
        let key = stored
            .key
            .as_ref()
            .ok_or("Jev needs an API key in Settings")?;
        Ok(Some(JevConfig {
            key: self.unseal("jev", key)?,
            model: stored.model,
        }))
    }

    pub fn view(&self, provider: &str) -> Result<ProviderSettingsResponse, String> {
        let stored = self.stored(provider)?;
        let (enabled, endpoint, model, has_key, revision, source) = match stored {
            Some((stored, revision)) => (
                stored.enabled,
                stored.endpoint,
                stored.model,
                stored.key.is_some(),
                revision,
                "settings",
            ),
            None if provider == "embeddings" => (
                env_value("TASKS_EMBEDDINGS_URL").is_some(),
                env_value("TASKS_EMBEDDINGS_URL").unwrap_or_default(),
                env_value("TASKS_EMBEDDINGS_MODEL").unwrap_or_default(),
                env_value("TASKS_EMBEDDINGS_API_KEY").is_some(),
                0,
                "environment",
            ),
            None => (
                env_value("TASKS_JEV_API_KEY").is_some(),
                JEV_ENDPOINT.into(),
                env_value("TASKS_JEV_MODEL").unwrap_or_else(|| "jev-latest".into()),
                env_value("TASKS_JEV_API_KEY").is_some(),
                0,
                "environment",
            ),
        };
        let resolved = if provider == "embeddings" {
            self.embeddings().map(|c| c.is_some())
        } else {
            self.jev().map(|c| c.is_some())
        };
        let (configured, notice) = match resolved {
            Ok(true) => (true, String::new()),
            Ok(false) => (false, "Provider is disabled.".into()),
            Err(error) => (false, error),
        };
        let endpoint = if !endpoint.is_empty() && validate_endpoint(&endpoint).is_err() {
            String::new()
        } else {
            endpoint
        };
        Ok(ProviderSettingsResponse {
            provider: provider.into(),
            enabled,
            endpoint,
            model,
            has_key,
            revision,
            source: source.into(),
            configured,
            notice,
        })
    }

    pub fn views(&self) -> Result<AiSettingsResponse, String> {
        Ok(AiSettingsResponse {
            embeddings: self.view("embeddings")?,
            jev: self.view("jev")?,
        })
    }

    pub fn prepare_update(
        &self,
        input: &SaveProviderInput,
        who: &str,
    ) -> Result<AppSettingDto, String> {
        validate_provider(&input.provider)?;
        let id = format!("provider:{}", input.provider);
        if self.revision(&id) != input.revision {
            return Err("Settings changed in another session. Reload before saving.".into());
        }
        let previous = self.view(&input.provider)?;
        let endpoint = if input.provider == "jev" {
            JEV_ENDPOINT.to_string()
        } else {
            input.endpoint.trim().to_string()
        };
        let model = input.model.trim().to_string();
        if input.enabled || !endpoint.is_empty() {
            validate_endpoint(&endpoint)?;
        }
        if input.enabled || !model.is_empty() {
            validate_model(&model)?;
        }
        let replacement = input
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty());
        if replacement.is_some() && input.clear_key {
            return Err("Choose replacing or removing the key, not both.".into());
        }
        if endpoint != previous.endpoint
            && previous.has_key
            && replacement.is_none()
            && !input.clear_key
        {
            return Err(
                "Changing the provider endpoint requires replacing or explicitly removing its key."
                    .into(),
            );
        }
        let key = if input.clear_key {
            None
        } else if let Some(key) = replacement {
            if key.len() > 8192 || !key.is_ascii() || key.chars().any(char::is_control) {
                return Err("Provider key is invalid.".into());
            }
            Some(self.seal(&input.provider, key)?)
        } else if let Some((stored, _)) = self.stored(&input.provider)? {
            stored.key
        } else {
            let name = if input.provider == "embeddings" {
                "TASKS_EMBEDDINGS_API_KEY"
            } else {
                "TASKS_JEV_API_KEY"
            };
            env_value(name)
                .map(|key| self.seal(&input.provider, &key))
                .transpose()?
        };
        if input.enabled && input.provider == "jev" && key.is_none() {
            return Err("Add a Jev API key before enabling the provider.".into());
        }
        let value = serde_json::to_string(&StoredProvider {
            enabled: input.enabled,
            endpoint,
            model,
            key,
        })
        .map_err(|_| "provider settings could not be encoded")?;
        Ok(AppSettingDto {
            id,
            value,
            revision: input.revision + 1,
            updated_by: who.into(),
        })
    }
}

pub async fn save_provider(
    app: &AppContext,
    input: &SaveProviderInput,
    who: &str,
) -> Result<AiSettingsResponse, String> {
    let _guard = app.configuration.mutations.lock().await;
    let row = app.configuration.prepare_update(input, who)?;
    app.settings_repo.upsert(&row).await?;
    app.configuration.install(row);
    app.configuration.views()
}

fn validate_provider(provider: &str) -> Result<(), String> {
    if matches!(provider, "embeddings" | "jev") {
        Ok(())
    } else {
        Err("Unknown AI provider.".into())
    }
}

fn validate_model(model: &str) -> Result<(), String> {
    if model.is_empty() || model.len() > 256 || model.chars().any(char::is_control) {
        Err("Model must contain 1..256 characters without control characters.".into())
    } else {
        Ok(())
    }
}

pub fn validate_endpoint(endpoint: &str) -> Result<(), String> {
    let url = url::Url::parse(endpoint).map_err(|_| "Enter a full HTTP(S) embeddings endpoint.")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "Endpoint must use HTTP(S) with a host, without embedded credentials or a fragment."
                .into(),
        );
    }
    if url.query_pairs().any(|(key, _)| {
        matches!(
            key.to_ascii_lowercase().as_str(),
            "key" | "api_key" | "apikey" | "api-key" | "token" | "access_token" | "secret"
        )
    }) {
        return Err("Enter credentials in the API key field, not in the endpoint URL.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> SaveProviderInput {
        SaveProviderInput {
            provider: "embeddings".into(),
            enabled: true,
            endpoint: "https://embedding.example.test/v1/embeddings".into(),
            model: "test-model-v1".into(),
            api_key: Some("synthetic-private-key".into()),
            clear_key: false,
            revision: 0,
        }
    }

    #[test]
    fn credentials_are_authenticated_encrypted_durable_and_never_returned() {
        let config = Configuration::new("synthetic-master", vec![]);
        let row = config
            .prepare_update(&input(), "owner@example.test")
            .unwrap();
        assert!(!row.value.to_string().contains("synthetic-private-key"));
        let restored = Configuration::new("synthetic-master", vec![row.clone()]);
        assert_eq!(
            restored.embeddings().unwrap().unwrap().key.as_deref(),
            Some("synthetic-private-key")
        );
        let response = serde_json::to_string(&restored.views().unwrap()).unwrap();
        assert!(!response.contains("synthetic-private-key"));
        assert!(!response.contains("ciphertext"));
        let wrong_key = Configuration::new("different-master", vec![row]);
        assert!(wrong_key.embeddings().is_err());
        let sealed = config.seal("embeddings", "synthetic-private-key").unwrap();
        assert!(config.unseal("jev", &sealed).is_err());
    }

    #[test]
    fn disabling_removal_and_stale_edits_are_explicit() {
        let config = Configuration::new("test", vec![]);
        config.install(config.prepare_update(&input(), "owner").unwrap());
        assert!(config.prepare_update(&input(), "stale-editor").is_err());
        let mut next = input();
        next.revision = 1;
        next.api_key = None;
        next.endpoint = "https://another.example.test/embeddings".into();
        assert!(config.prepare_update(&next, "owner").is_err());
        next.clear_key = true;
        next.enabled = false;
        config.install(config.prepare_update(&next, "owner").unwrap());
        assert!(config.embeddings().unwrap().is_none());
        assert!(!config.view("embeddings").unwrap().has_key);
    }

    #[test]
    fn invalid_endpoints_and_key_in_urls_are_rejected() {
        for endpoint in [
            "file:///etc/passwd",
            "https://user:pass@host/",
            "https://host/path?api_key=secret",
            "https://host/path#secret",
        ] {
            assert!(validate_endpoint(endpoint).is_err());
        }
        assert!(validate_endpoint("http://embeddings:8080/v1/embeddings").is_ok());
    }
}
