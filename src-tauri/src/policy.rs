//! ~/.friction/rules.toml: loading, hot reload state, and layering the file's
//! settings on top of the app's own settings.

use crate::config::{data_dir, Config};
use crate::rules_file::{self, RulesFile};
use serde::Serialize;
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RulesFileStatus {
    pub path: String,
    pub ok: bool,
    pub error: Option<String>,
    pub custom_rules: usize,
    /// Settings currently controlled by the file (UI keys).
    pub overrides: Vec<String>,
}

#[derive(Default)]
pub struct Loaded {
    pub file: RulesFile,
    pub status: RulesFileStatus,
    pub mtime: Option<SystemTime>,
}

pub fn path() -> PathBuf {
    data_dir().join("rules.toml")
}

pub fn ensure_template() {
    let p = path();
    let replace = match std::fs::read_to_string(&p) {
        Err(_) => true,
        // An untouched template from an older version (only comments, and
        // still mentions a setting that no longer exists): safe to refresh.
        Ok(t) => {
            t.contains("only_approved_merchants")
                && t.lines().all(|l| l.trim().is_empty() || l.trim_start().starts_with('#'))
        }
    };
    if replace {
        let _ = std::fs::write(&p, rules_file::TEMPLATE);
    }
}

fn mtime() -> Option<SystemTime> {
    std::fs::metadata(path()).and_then(|m| m.modified()).ok()
}

/// True when the file changed since `prev` was loaded.
pub fn changed(prev: &Loaded) -> bool {
    mtime() != prev.mtime
}

/// Reads and parses the file. On a parse error the previous good rules are kept.
pub fn reload(prev: &Loaded) -> Loaded {
    let p = path();
    let m = mtime();
    let text = std::fs::read_to_string(&p).unwrap_or_default();
    match rules_file::parse(&text) {
        Ok(file) => {
            let status = RulesFileStatus {
                path: p.to_string_lossy().to_string(),
                ok: true,
                error: None,
                custom_rules: file.custom_rule_count(),
                overrides: overrides(&file),
            };
            Loaded { file, status, mtime: m }
        }
        Err(e) => Loaded {
            file: prev.file.clone(),
            status: RulesFileStatus {
                path: p.to_string_lossy().to_string(),
                ok: false,
                error: Some(e),
                custom_rules: prev.file.custom_rule_count(),
                overrides: overrides(&prev.file),
            },
            mtime: m,
        },
    }
}

pub fn overrides(f: &RulesFile) -> Vec<String> {
    let mut o = Vec::new();
    if f.budget.daily_ai_spend.is_some() {
        o.push("dailyAiCap".into());
    }
    let r = &f.rules;
    for (set, key) in [
        (r.hold_payments.is_some(), "holdPayments"),
        (r.hold_checkout_pages.is_some(), "holdCheckoutPages"),
        (r.block_secrets.is_some(), "blockSecretLeaks"),
        (r.save_request_bodies.is_some(), "logPayloadsLocally"),
    ] {
        if set {
            o.push(key.into());
        }
    }
    o
}

fn agent_override<'a>(f: &'a RulesFile, agent: &str) -> Option<&'a rules_file::AgentOverride> {
    f.agents.iter().find(|(k, _)| k.eq_ignore_ascii_case(agent)).map(|(_, v)| v)
}

/// App settings with rules.toml applied on top.
pub fn effective(cfg: &Config, f: &RulesFile) -> Config {
    let mut c = cfg.clone();
    if let Some(cap) = f.budget.daily_ai_spend {
        c.daily_ai_cap = Some(cap);
    }
    let r = &f.rules;
    if let Some(v) = r.hold_payments {
        c.rules.hold_payments = v;
    }
    if let Some(v) = r.hold_checkout_pages {
        c.rules.hold_checkout_pages = v;
    }
    if let Some(v) = r.block_secrets {
        c.rules.block_secret_leaks = v;
    }
    if let Some(v) = r.save_request_bodies {
        c.rules.log_payloads_locally = v;
    }
    c
}

/// Per-agent daily AI cap from the file, if any.
pub fn agent_ai_cap(f: &RulesFile, agent: &str) -> Option<f64> {
    agent_override(f, agent).and_then(|a| a.daily_ai_spend)
}

pub fn price_override(f: &RulesFile, model: &str) -> Option<crate::spend::Price> {
    f.pricing.iter().find(|(k, _)| k.eq_ignore_ascii_case(model)).map(|(_, p)| crate::spend::Price {
        input: p.input,
        output: p.output,
        cached_input: p.cached_input,
        cache_write: None,
    })
}
