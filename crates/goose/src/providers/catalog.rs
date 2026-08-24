use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::canonical::CanonicalModelRegistry;

const PROVIDER_METADATA_JSON: &str = include_str!("canonical/data/provider_metadata.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderMetadataEntry {
    pub id: String,
    pub display_name: String,
    pub npm: Option<String>,
    pub api: Option<String>,
    pub doc: Option<String>,
    pub env: Vec<String>,
    pub model_count: usize,
}

static PROVIDER_METADATA: Lazy<HashMap<String, ProviderMetadataEntry>> = Lazy::new(|| {
    serde_json::from_str::<Vec<ProviderMetadataEntry>>(PROVIDER_METADATA_JSON)
        .unwrap_or_else(|e| {
            eprintln!("Failed to parse provider metadata: {}", e);
            Vec::new()
        })
        .into_iter()
        .map(|p| (p.id.clone(), p))
        .collect()
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderFormat {
    OpenAI,
    Anthropic,
    Ollama,
}

impl ProviderFormat {
    pub fn as_str(&self) -> &str {
        match self {
            ProviderFormat::OpenAI => "openai",
            ProviderFormat::Anthropic => "anthropic",
            ProviderFormat::Ollama => "ollama",
        }
    }
}

impl std::str::FromStr for ProviderFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" | "openai_compatible" => Ok(ProviderFormat::OpenAI),
            "anthropic" | "anthropic_compatible" => Ok(ProviderFormat::Anthropic),
            "ollama" | "ollama_compatible" => Ok(ProviderFormat::Ollama),
            _ => Err(format!("unknown provider format: {}", s)),
        }
    }
}

fn detect_format_from_npm(npm: &str) -> Option<ProviderFormat> {
    if npm.contains("openai") {
        Some(ProviderFormat::OpenAI)
    } else if npm.contains("anthropic") {
        Some(ProviderFormat::Anthropic)
    } else if npm.contains("ollama") {
        Some(ProviderFormat::Ollama)
    } else {
        None
    }
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ProviderCatalogEntry {
    pub id: String,
    pub name: String,
    pub format: String,
    pub api_url: String,
    pub model_count: usize,
    pub doc_url: String,
    pub env_var: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ProviderTemplate {
    pub id: String,
    pub name: String,
    pub format: String,
    pub api_url: String,
    pub models: Vec<ModelTemplate>,
    pub supports_streaming: bool,
    pub env_var: String,
    pub doc_url: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ModelTemplate {
    pub id: String,
    pub name: String,
    pub context_limit: usize,
    pub capabilities: ModelCapabilities,
    pub deprecated: bool,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ModelCapabilities {
    pub tool_call: bool,
    pub reasoning: bool,
    pub attachment: bool,
    pub temperature: bool,
}

pub async fn get_providers_by_format(format: ProviderFormat) -> Vec<ProviderCatalogEntry> {
    let native_provider_ids = super::init::providers()
        .await
        .into_iter()
        .map(|(metadata, _)| metadata.name)
        .collect::<std::collections::HashSet<_>>();

    let mut entries: Vec<ProviderCatalogEntry> = PROVIDER_METADATA
        .values()
        .filter_map(|metadata| {
            if native_provider_ids.contains(&metadata.id) {
                return None;
            }

            let npm = metadata.npm.as_ref()?;
            let detected_format = detect_format_from_npm(npm)?;

            if detected_format != format {
                return None;
            }

            let api_url = metadata.api.as_ref()?.clone();

            let env_var = metadata.env.first().cloned().unwrap_or_else(|| {
                format!("{}_API_KEY", metadata.id.to_uppercase().replace('-', "_"))
            });

            Some(ProviderCatalogEntry {
                id: metadata.id.clone(),
                name: metadata.display_name.clone(),
                format: detected_format.as_str().to_string(),
                api_url,
                model_count: metadata.model_count,
                doc_url: metadata.doc.clone().unwrap_or_default(),
                env_var,
            })
        })
        .collect();

    // Sort by name
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

pub fn get_provider_template(provider_id: &str) -> Option<ProviderTemplate> {
    let metadata = PROVIDER_METADATA.get(provider_id)?;

    let npm = metadata.npm.as_ref()?;
    let format = detect_format_from_npm(npm)?;

    let api_url = metadata.api.as_ref()?.clone();

    let models: Vec<ModelTemplate> = CanonicalModelRegistry::bundled()
        .ok()
        .map(|registry| {
            registry
                .get_all_models_for_provider(provider_id)
                .into_iter()
                .map(|model| {
                    // Extract just the model ID (without provider prefix)
                    let model_id = model
                        .id
                        .strip_prefix(&format!("{}/", provider_id))
                        .unwrap_or(&model.id)
                        .to_string();

                    ModelTemplate {
                        id: model_id,
                        name: model.name.clone(),
                        context_limit: model.limit.context,
                        capabilities: ModelCapabilities {
                            tool_call: model.tool_call,
                            reasoning: model.reasoning.unwrap_or(false),
                            attachment: model.attachment.unwrap_or(false),
                            temperature: model.temperature.unwrap_or(false),
                        },
                        deprecated: false, // Canonical models don't have deprecated flag
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let env_var = metadata
        .env
        .first()
        .cloned()
        .unwrap_or_else(|| format!("{}_API_KEY", provider_id.to_uppercase().replace('-', "_")));

    Some(ProviderTemplate {
        id: metadata.id.clone(),
        name: metadata.display_name.clone(),
        format: format.as_str().to_string(),
        api_url,
        models,
        supports_streaming: true, // Default to true
        env_var,
        doc_url: metadata.doc.clone().unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `zai` used to be catalog-only (an "add a custom provider" entry). It is now
    /// a NATIVE provider (`providers/zai.rs`, registered in `init.rs`), so
    /// `get_providers_by_format` deliberately filters it out — offering a
    /// built-in provider as a custom one would create two ways to configure the
    /// same thing. The template lookup is NOT filtered and still resolves, which
    /// is what the custom-provider flow for the Coding Plan endpoint relies on.
    #[tokio::test]
    async fn zai_is_native_and_therefore_not_in_the_custom_catalog() {
        let native = super::super::init::providers()
            .await
            .into_iter()
            .map(|(m, _)| m.name)
            .collect::<std::collections::HashSet<_>>();
        assert!(
            native.contains("zai"),
            "zai must be registered as a native provider"
        );

        let openai_providers = get_providers_by_format(ProviderFormat::OpenAI).await;
        assert!(
            !openai_providers.iter().any(|p| p.id == "zai"),
            "a native provider must not also be offered as a custom-catalog entry"
        );

        // The separate Coding Plan endpoint is not a native provider, so it stays
        // in the catalog for operators who buy that plan.
        assert!(
            openai_providers.iter().any(|p| p.id == "zai-coding-plan"),
            "zai-coding-plan should remain in the catalog"
        );

        // Templates are keyed off the bundled provider metadata, not the catalog
        // filter, so the zai template still resolves with its models.
        let template = get_provider_template("zai").expect("z.ai should have a template");
        assert!(
            !template.models.is_empty(),
            "z.ai template should have models"
        );
    }
}
