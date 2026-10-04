//! User configuration (budget, rules, merchant allowlist, SafePrompt key).
//! Persisted to ~/.friction/config.json so the proxy enforces the user's
//! policy from the moment the app starts, before the UI has loaded.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Rules {
    /// Hold requests to known payment APIs (Stripe, PayPal, ...) for approval.
    pub hold_payments: bool,
    /// Also hold POSTs to checkout-style paths on any site.
    pub hold_checkout_pages: bool,
    /// Secrets are always caught; when true they're blocked instead of held.
    pub block_secret_leaks: bool,
    /// Keep request and response bodies (Activity and ~/.friction/payloads).
    pub log_payloads_locally: bool,
}

impl Default for Rules {
    fn default() -> Self {
        Self { hold_payments: true, hold_checkout_pages: true, block_secret_leaks: false, log_payloads_locally: true }
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub rules: Rules,
    /// Set once real agent traffic has passed through (drives the setup checklist).
    pub seen_traffic: bool,
    /// Daily AI API spend limit in USD; AI calls are blocked once reached.
    pub daily_ai_cap: Option<f64>,
}

/// What the UI sees.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicConfig {
    pub rules: Rules,
    pub seen_traffic: bool,
    pub daily_ai_cap: Option<f64>,
    /// Settings controlled by ~/.friction/rules.toml (filled in by AppState).
    pub file_overrides: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigPatch {
    pub rules: Option<Rules>,
}

impl Config {
    pub fn public(&self) -> PublicConfig {
        PublicConfig {
            rules: self.rules.clone(),
            seen_traffic: self.seen_traffic,
            daily_ai_cap: self.daily_ai_cap,
            file_overrides: Vec::new(),
        }
    }

    pub fn apply(&mut self, patch: ConfigPatch) {
        if let Some(r) = patch.rules {
            self.rules = r;
        }
    }
}

pub fn data_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let dir = PathBuf::from(home).join(".friction");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Write a file readable only by the current user.
pub fn write_private(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(contents)?;
    }
    std::fs::rename(tmp, path)
}

pub fn load() -> Config {
    let path = data_dir().join("config.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(cfg: &Config) {
    let path = data_dir().join("config.json");
    if let Ok(json) = serde_json::to_vec_pretty(cfg) {
        if let Err(e) = write_private(&path, &json) {
            eprintln!("[friction] failed to save config: {e}");
        }
    }
}
