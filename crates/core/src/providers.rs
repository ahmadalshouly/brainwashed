//! Cloud providers people connect with their own API key, for when a local
//! model isn't wanted or isn't enough. Anything with an OpenAI-compatible
//! chat API works: OpenAI, Anthropic, Gemini, OpenRouter, Groq, Mistral,
//! DeepSeek, or Ollama and LM Studio on another machine.
//!
//! Keys stay in `providers.json` on this computer (readable only by its
//! owner) and are never sent to devices.

use crate::engine::Event;
use crate::{Engine, Error, Result};
use brainwashed_runtime::chat::{self, Endpoint};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Model id prefix that picks the local model: `local`, or anything without
/// a provider.
pub const LOCAL_MODEL: &str = "local";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    /// Short name used in model ids, e.g. `openai` in `openai/gpt-4o`.
    pub id: String,
    pub name: String,
    /// API base that `/chat/completions` hangs off.
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Models offered in the chat.
    #[serde(default)]
    pub models: Vec<String>,
    /// Members may chat with these models too, not only admins.
    #[serde(default = "yes")]
    pub members: bool,
}

fn yes() -> bool {
    true
}

/// A provider as admins see it: the key is never sent back.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub models: Vec<String>,
    pub members: bool,
    pub key_set: bool,
    /// Last four characters of the key, to tell keys apart.
    pub key_hint: Option<String>,
}

impl From<&Provider> for ProviderInfo {
    fn from(p: &Provider) -> Self {
        let key = p.api_key.as_deref().filter(|k| !k.is_empty());
        ProviderInfo {
            id: p.id.clone(),
            name: p.name.clone(),
            base_url: p.base_url.clone(),
            models: p.models.clone(),
            members: p.members,
            key_set: key.is_some(),
            key_hint: key.map(|k| {
                let tail: String = k
                    .chars()
                    .rev()
                    .take(4)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                format!("…{tail}")
            }),
        }
    }
}

/// A model someone can pick in the chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatModel {
    /// `local`, or `<provider id>/<model>`.
    pub id: String,
    pub name: String,
    /// Provider name; None for the model running on this computer.
    pub provider: Option<String>,
    /// Can look at pictures. Unknown for cloud models, so assumed.
    pub vision: bool,
    /// Messages leave this computer.
    pub cloud: bool,
}

impl Engine {
    fn providers_file(&self) -> PathBuf {
        self.data_dir().join("providers.json")
    }

    pub fn providers(&self) -> Vec<Provider> {
        crate::store::load(&self.providers_file())
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    fn save_providers(&self, providers: &[Provider]) -> Result<()> {
        let path = self.providers_file();
        crate::store::save(&path, &providers)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        self.emit(Event::ModelsChanged);
        Ok(())
    }

    /// Adds or updates a provider. A provider saved without a key keeps the
    /// key it had.
    pub fn save_provider(&self, mut provider: Provider) -> Result<Provider> {
        provider.id = provider.id.trim().to_ascii_lowercase();
        provider.name = provider.name.trim().to_string();
        provider.base_url = provider.base_url.trim().trim_end_matches('/').to_string();
        if provider.id.is_empty()
            || provider.id == LOCAL_MODEL
            || !provider
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(Error::Invalid(
                "the provider id must be letters, numbers, - or _".into(),
            ));
        }
        if provider.name.is_empty() {
            provider.name = provider.id.clone();
        }
        if !(provider.base_url.starts_with("https://") || provider.base_url.starts_with("http://"))
        {
            return Err(Error::Invalid(
                "the API address must start with https:// or http://".into(),
            ));
        }
        provider
            .models
            .iter_mut()
            .for_each(|m| *m = m.trim().to_string());
        provider.models.retain(|m| !m.is_empty());
        provider.models.dedup();
        let mut all = self.providers();
        let old_key = all
            .iter()
            .find(|p| p.id == provider.id)
            .and_then(|p| p.api_key.clone());
        match provider.api_key.as_deref().map(str::trim) {
            None => provider.api_key = old_key,
            Some("") => provider.api_key = None,
            Some(k) => provider.api_key = Some(k.to_string()),
        }
        all.retain(|p| p.id != provider.id);
        all.push(provider.clone());
        self.save_providers(&all)?;
        Ok(provider)
    }

    pub fn delete_provider(&self, id: &str) -> Result<()> {
        let mut all = self.providers();
        let before = all.len();
        all.retain(|p| p.id != id);
        if all.len() == before {
            return Err(Error::Invalid(format!("no provider `{id}`")));
        }
        self.save_providers(&all)
    }

    /// Models a provider offers. `key` overrides the saved key, to try one
    /// before saving it.
    pub async fn provider_models(
        &self,
        base_url: &str,
        key: Option<&str>,
        saved_id: Option<&str>,
    ) -> Result<Vec<String>> {
        let saved_key = saved_id.and_then(|id| {
            self.providers()
                .into_iter()
                .find(|p| p.id == id)
                .and_then(|p| p.api_key)
        });
        let endpoint = Endpoint {
            base_url: base_url.trim().trim_end_matches('/').to_string(),
            api_key: key
                .map(str::to_string)
                .filter(|k| !k.is_empty())
                .or(saved_key),
            model: None,
            llama: false,
            ask_user: false,
        };
        Ok(chat::list_models(&self.inner.client, &endpoint).await?)
    }

    /// What the chat can talk to: the local model when one is loaded, and
    /// each provider model this device may use.
    pub fn chat_models(&self, admin: bool) -> Vec<ChatModel> {
        let mut out = Vec::new();
        if let crate::EngineState::Ready { model } = self.state() {
            let installed = self.models().into_iter().find(|m| m.id == model);
            out.push(ChatModel {
                id: LOCAL_MODEL.into(),
                name: installed.as_ref().map_or(model.clone(), |m| m.name.clone()),
                provider: None,
                vision: self.vision(),
                cloud: false,
            });
        }
        for p in self.providers() {
            if !admin && !p.members {
                continue;
            }
            for m in &p.models {
                out.push(ChatModel {
                    id: format!("{}/{m}", p.id),
                    name: m.clone(),
                    provider: Some(p.name.clone()),
                    vision: true,
                    cloud: true,
                });
            }
        }
        out
    }

    /// Finds the provider and model behind a chat model id.
    pub(crate) fn provider_endpoint(&self, id: &str, admin: bool) -> Result<Endpoint> {
        let (provider_id, model) = id
            .split_once('/')
            .ok_or_else(|| Error::Invalid(format!("no model `{id}`")))?;
        let provider = self
            .providers()
            .into_iter()
            .find(|p| p.id == provider_id)
            .ok_or_else(|| Error::Invalid(format!("no provider `{provider_id}`")))?;
        if !admin && !(provider.members && provider.models.iter().any(|m| m == model)) {
            return Err(Error::Invalid(format!(
                "Only admins can use {model} from {}.",
                provider.name
            )));
        }
        Ok(Endpoint {
            base_url: provider.base_url,
            api_key: provider.api_key,
            model: Some(model.to_string()),
            llama: false,
            ask_user: false,
        })
    }
}
