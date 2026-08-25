//! Which model answers, where it answers, and where that choice is saved.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use crate::config::{LLM_MODEL, OLLAMA_CHAT_ENDPOINT};

const CONFIG_FILE_ENV: &str = "LG_CONFIG_FILE";
const CONFIG_MODEL_KEY: &str = "llm_model";
const CONFIG_PROVIDER_KEY: &str = "llm_provider";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmProvider {
    Ollama,
}

impl LlmProvider {
    pub const ALL: [Self; 1] = [Self::Ollama];

    pub fn label(self) -> &'static str {
        "ollama"
    }

    fn config_value(self) -> &'static str {
        "ollama"
    }

    fn from_config(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "ollama" | "ollama-server" | "ollama_server" | "llama" | "llama-server"
            | "llama_server" | "llamacpp" | "llama.cpp" => Some(Self::Ollama),
            _ => None,
        }
    }

    fn default_endpoint(self) -> &'static str {
        OLLAMA_CHAT_ENDPOINT
    }

    fn endpoint_env(self) -> Option<String> {
        std::env::var("LG_OLLAMA_CHAT_ENDPOINT")
            .or_else(|_| std::env::var("LG_OLLAMA_URL"))
            .ok()
            .map(|endpoint| endpoint.trim().to_string())
            .filter(|endpoint| !endpoint.is_empty())
            .map(|endpoint| normalize_ollama_chat_endpoint(&endpoint))
    }
}

pub fn current_model() -> String {
    env_model()
        .or_else(saved_model)
        .or_else(first_available_ollama_model)
        .unwrap_or_else(|| LLM_MODEL.to_owned())
}

pub fn current_provider() -> LlmProvider {
    env_provider()
        .or_else(saved_provider)
        .unwrap_or(LlmProvider::Ollama)
}

pub fn current_endpoint() -> String {
    endpoint_for_provider(current_provider())
}

pub fn endpoint_for_provider(provider: LlmProvider) -> String {
    provider
        .endpoint_env()
        .unwrap_or_else(|| provider.default_endpoint().to_owned())
}

fn normalize_ollama_chat_endpoint(endpoint: &str) -> String {
    let endpoint = endpoint.trim().trim_end_matches('/');
    if endpoint.ends_with("/api/chat") {
        endpoint.to_string()
    } else if endpoint.ends_with("/api") {
        format!("{endpoint}/chat")
    } else {
        format!("{endpoint}/api/chat")
    }
}

fn ollama_tags_endpoint() -> String {
    let endpoint = current_endpoint();
    let base = endpoint.trim_end_matches("/api/chat").trim_end_matches('/');
    format!("{base}/api/tags")
}

#[derive(Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaTagModel>,
}

#[derive(Deserialize)]
struct OllamaTagModel {
    name: String,
}

fn first_available_ollama_model() -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .ok()?;
    client
        .get(ollama_tags_endpoint())
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .json::<OllamaTagsResponse>()
        .ok()?
        .models
        .into_iter()
        .map(|model| model.name.trim().to_string())
        .find(|name| !name.is_empty())
}

pub fn env_model_active() -> bool {
    env_model().is_some()
}

pub fn env_provider_active() -> bool {
    env_provider().is_some()
}

pub fn config_file_display() -> String {
    config_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "$HOME/.config/lg/config".to_string())
}

pub fn save_llm_settings(model: &str, provider: LlmProvider) -> Result<()> {
    let model = model.trim();
    if model.is_empty() {
        anyhow::bail!("model is empty");
    }
    if model.chars().any(|ch| ch == '\n' || ch == '\r') {
        anyhow::bail!("model must fit on one line");
    }
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut entries = read_config_entries(&path);
    set_config_entry(&mut entries, CONFIG_MODEL_KEY, model);
    set_config_entry(&mut entries, CONFIG_PROVIDER_KEY, provider.config_value());
    fs::write(&path, render_config_entries(&entries))
        .with_context(|| format!("failed to write {}", path.display()))
}

pub fn clear_saved_llm_settings() -> Result<()> {
    let path = config_path()?;
    let mut entries = read_config_entries(&path);
    let before = entries.len();
    entries.retain(|(key, _)| key != CONFIG_MODEL_KEY && key != CONFIG_PROVIDER_KEY);
    if entries.len() == before && !path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&path, render_config_entries(&entries))
        .with_context(|| format!("failed to write {}", path.display()))
}

fn env_model() -> Option<String> {
    std::env::var("LG_LLM_MODEL")
        .ok()
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
}

fn env_provider() -> Option<LlmProvider> {
    std::env::var("LG_LLM_PROVIDER")
        .ok()
        .and_then(|provider| LlmProvider::from_config(&provider))
}

fn saved_model() -> Option<String> {
    let path = config_path().ok()?;
    read_config_entries(&path)
        .into_iter()
        .find_map(|(key, value)| (key == CONFIG_MODEL_KEY && !value.is_empty()).then_some(value))
}

fn saved_provider() -> Option<LlmProvider> {
    let path = config_path().ok()?;
    read_config_entries(&path)
        .into_iter()
        .find_map(|(key, value)| {
            (key == CONFIG_PROVIDER_KEY)
                .then(|| LlmProvider::from_config(&value))
                .flatten()
        })
}

fn config_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os(CONFIG_FILE_ENV)
        && !path.is_empty()
    {
        return Ok(PathBuf::from(path));
    }
    let Some(home) = std::env::var_os("HOME") else {
        anyhow::bail!("HOME is not set");
    };
    Ok(PathBuf::from(home).join(".config/lg/config"))
}

fn read_config_entries(path: &std::path::Path) -> Vec<(String, String)> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

fn set_config_entry(entries: &mut Vec<(String, String)>, key: &str, value: &str) {
    if let Some((_, existing)) = entries.iter_mut().find(|(candidate, _)| candidate == key) {
        *existing = value.to_string();
    } else {
        entries.push((key.to_string(), value.to_string()));
    }
}

fn render_config_entries(entries: &[(String, String)]) -> String {
    let mut out = String::new();
    for (key, value) in entries {
        out.push_str(key);
        out.push('=');
        out.push_str(value);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llm_provider_parses_ollama_aliases() {
        assert_eq!(
            LlmProvider::from_config("ollama"),
            Some(LlmProvider::Ollama)
        );
        assert_eq!(
            LlmProvider::from_config("llama-server"),
            Some(LlmProvider::Ollama)
        );
        assert_eq!(
            LlmProvider::from_config("llama.cpp"),
            Some(LlmProvider::Ollama)
        );
        assert_eq!(LlmProvider::from_config("unsupported"), None);
    }

    #[test]
    fn ollama_url_env_accepts_base_url() {
        assert_eq!(
            normalize_ollama_chat_endpoint("http://localhost:11434"),
            "http://localhost:11434/api/chat"
        );
        assert_eq!(
            normalize_ollama_chat_endpoint("http://localhost:11434/api/chat"),
            "http://localhost:11434/api/chat"
        );
        assert_eq!(
            normalize_ollama_chat_endpoint("http://localhost:11434/api"),
            "http://localhost:11434/api/chat"
        );
    }
}
