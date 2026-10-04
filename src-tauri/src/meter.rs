//! Daily AI API spend, persisted to ~/.friction/spend.json.

use crate::config::{data_dir, write_private};
use crate::spend::Usage;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const KEEP_DAYS: usize = 90;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct ModelDay {
    provider: String,
    model: String,
    calls: u64,
    input_tokens: u64,
    output_tokens: u64,
    cached_input_tokens: u64,
    cost_usd: f64,
    unpriced_calls: u64,
    estimated_calls: u64,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct AgentDay {
    calls: u64,
    cost_usd: f64,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Day {
    models: BTreeMap<String, ModelDay>,
    agents: BTreeMap<String, AgentDay>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Ledger {
    days: BTreeMap<String, Day>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendRow {
    pub provider: String,
    pub model: String,
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: Option<f64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendAgentRow {
    pub agent: String,
    pub calls: u64,
    pub cost_usd: f64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendDay {
    pub date: String,
    pub cost_usd: f64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendSummary {
    pub today: f64,
    pub today_estimated: bool,
    pub cap: Option<f64>,
    pub cap_reached: bool,
    pub days: Vec<SpendDay>,
    pub by_model: Vec<SpendRow>,
    pub by_agent: Vec<SpendAgentRow>,
    pub prices_as_of: String,
    pub unpriced_models: Vec<String>,
    pub cap_from_rules_file: bool,
}

pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn path() -> std::path::PathBuf {
    data_dir().join("spend.json")
}

impl Ledger {
    pub fn load() -> Self {
        std::fs::read_to_string(path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_vec(self) {
            let _ = write_private(&path(), &json);
        }
    }

    pub fn record(&mut self, agent: &str, provider: &str, model: &str, usage: &Usage, cost: Option<f64>) {
        let day = self.days.entry(today()).or_default();
        let m = day.models.entry(format!("{provider}/{model}")).or_default();
        m.provider = provider.to_string();
        m.model = model.to_string();
        m.calls += 1;
        m.input_tokens += usage.input_tokens;
        m.output_tokens += usage.output_tokens;
        m.cached_input_tokens += usage.cached_input_tokens;
        match cost {
            Some(c) => m.cost_usd += c,
            None => m.unpriced_calls += 1,
        }
        if usage.estimated {
            m.estimated_calls += 1;
        }
        let a = day.agents.entry(agent.to_string()).or_default();
        a.calls += 1;
        a.cost_usd += cost.unwrap_or(0.0);
        while self.days.len() > KEEP_DAYS {
            let first = self.days.keys().next().cloned();
            if let Some(k) = first {
                self.days.remove(&k);
            }
        }
        self.save();
    }

    pub fn clear(&mut self) {
        self.days.clear();
        self.save();
    }

    pub fn today_total(&self) -> f64 {
        self.days.get(&today()).map(|d| d.agents.values().map(|a| a.cost_usd).sum()).unwrap_or(0.0)
    }

    pub fn agent_today(&self, agent: &str) -> f64 {
        self.days
            .get(&today())
            .and_then(|d| d.agents.get(agent))
            .map(|a| a.cost_usd)
            .unwrap_or(0.0)
    }

    pub fn summary(&self, cap: Option<f64>, cap_from_rules_file: bool) -> SpendSummary {
        let t = today();
        let today_day = self.days.get(&t).cloned().unwrap_or_default();
        let total = self.today_total();
        let mut days = Vec::new();
        let now = chrono::Local::now();
        for i in (0..14).rev() {
            let d = (now - chrono::Duration::days(i)).format("%Y-%m-%d").to_string();
            let cost = self.days.get(&d).map(|x| x.agents.values().map(|a| a.cost_usd).sum()).unwrap_or(0.0);
            days.push(SpendDay { date: d, cost_usd: cost });
        }
        let mut by_model: Vec<SpendRow> = today_day
            .models
            .values()
            .map(|m| SpendRow {
                provider: m.provider.clone(),
                model: m.model.clone(),
                calls: m.calls,
                input_tokens: m.input_tokens + m.cached_input_tokens,
                output_tokens: m.output_tokens,
                cost_usd: (m.unpriced_calls < m.calls).then_some(m.cost_usd),
            })
            .collect();
        by_model.sort_by(|a, b| {
            b.cost_usd.unwrap_or(0.0).partial_cmp(&a.cost_usd.unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut by_agent: Vec<SpendAgentRow> = today_day
            .agents
            .iter()
            .map(|(k, a)| SpendAgentRow { agent: k.clone(), calls: a.calls, cost_usd: a.cost_usd })
            .collect();
        by_agent.sort_by(|a, b| b.cost_usd.partial_cmp(&a.cost_usd).unwrap_or(std::cmp::Ordering::Equal));
        let unpriced_models: Vec<String> =
            today_day.models.values().filter(|m| m.unpriced_calls > 0).map(|m| m.model.clone()).collect();
        SpendSummary {
            today: total,
            today_estimated: today_day.models.values().any(|m| m.estimated_calls > 0),
            cap,
            cap_reached: cap.is_some_and(|c| total >= c),
            days,
            by_model,
            by_agent,
            prices_as_of: crate::spend::PRICES_AS_OF.to_string(),
            unpriced_models,
            cap_from_rules_file,
        }
    }
}
