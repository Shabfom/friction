//! User-editable rules file (~/.friction/rules.toml).
//!
//! This module only parses text and evaluates matchers. The caller reads the
//! file from disk and decides how the values layer on top of the app settings.

use std::collections::BTreeMap;

/// Starter file written on first launch. Every option is present but commented out.
pub const TEMPLATE: &str = r###"# Friction rules. Changes apply as soon as you save.
# Anything set here overrides the app's settings.
# To turn a line on, remove the "# " in front of it.

# [budget]
# Block all AI API calls once today's spend reaches this many USD.
# daily_ai_spend = 10.0

# [rules]
# Hold payments to Stripe, PayPal, Square and other payment APIs for your OK.
# hold_payments = true
# Also hold POSTs to checkout, payment and purchase paths on any site.
# hold_checkout_pages = true
# Block leaked secrets without asking (when false they wait for you).
# block_secrets = false
# Keep request and response bodies for Activity and held requests.
# save_request_bodies = true

# Always block matching requests.
# [[block]]
# host = "*.pastebin.com"
# reason = "No paste sites"

# Always hold matching requests for your OK.
# [[hold]]
# method = "DELETE"
# host = "api.github.com"
# reason = "Deleting on GitHub"

# Never hold matching requests (leaked secrets are still caught).
# [[allow]]
# method = "POST"
# host = "api.stripe.com"
# agent = "billing-bot"

# Limits for one agent (its name from friction run --name or X-Friction-Agent).
# [agents.researcher]
# daily_ai_spend = 2.0

# Meter another AI API (an internal gateway or a self-hosted server).
# [[ai_endpoint]]
# host = "llm.internal.example"

# Price a model Friction doesn't know, in USD per 1M tokens.
# [pricing."my-finetune"]
# input = 3.0
# output = 15.0
# cached_input = 0.3
"###;

#[derive(Debug, Default, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RulesFile {
    #[serde(default)]
    pub budget: Budget,
    #[serde(default)]
    pub rules: Toggles,
    #[serde(default)]
    pub block: Vec<Matcher>,
    #[serde(default)]
    pub hold: Vec<Matcher>,
    #[serde(default)]
    pub allow: Vec<Matcher>,
    #[serde(default)]
    pub agents: BTreeMap<String, AgentOverride>,
    /// Extra hosts to meter as AI APIs: [[ai_endpoint]] host = "llm.internal.example"
    #[serde(default)]
    pub ai_endpoint: Vec<AiEndpoint>,
    #[serde(default)]
    pub pricing: BTreeMap<String, PriceOverride>,
}

#[derive(Debug, Default, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    #[serde(default)]
    pub daily_ai_spend: Option<f64>,
}

#[derive(Debug, Default, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Toggles {
    #[serde(default)]
    pub hold_payments: Option<bool>,
    #[serde(default)]
    pub hold_checkout_pages: Option<bool>,
    #[serde(default)]
    pub block_secrets: Option<bool>,
    #[serde(default)]
    pub save_request_bodies: Option<bool>,
}

#[derive(Debug, Default, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentOverride {
    #[serde(default)]
    pub daily_ai_spend: Option<f64>,
}

/// USD per 1M tokens.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriceOverride {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cached_input: Option<f64>,
}

#[derive(Debug, Default, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Matcher {
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Default, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiEndpoint {
    pub host: String,
}

impl RulesFile {
    /// True when `host` matches one of the [[ai_endpoint]] host globs.
    pub fn is_ai_endpoint(&self, host: &str) -> bool {
        self.ai_endpoint.iter().any(|e| {
            !e.host.trim().is_empty()
                && Matcher { host: Some(e.host.clone()), ..Default::default() }.matches("", "", host, "/")
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Block(String),
    Hold(String),
    Allow,
}

/// Returns the trimmed value when the field is set and not blank.
fn field(v: &Option<String>) -> Option<&str> {
    match v {
        Some(s) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        }
        None => None,
    }
}

/// Iterative wildcard match where '*' matches any run of characters.
/// Runs in O(pattern * text) worst case, never exponential.
fn glob_chars(pattern: &[char], text: &[char]) -> bool {
    let mut p = 0usize;
    let mut t = 0usize;
    let mut star: Option<usize> = None;
    let mut mark = 0usize;
    while t < text.len() {
        if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            mark = t;
            p += 1;
        } else if p < pattern.len() && pattern[p] == text[t] {
            p += 1;
            t += 1;
        } else if let Some(s) = star {
            p = s + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

fn glob(pattern: &str, text: &str, ignore_case: bool) -> bool {
    let (pc, tc): (Vec<char>, Vec<char>) = if ignore_case {
        (
            pattern.to_lowercase().chars().collect(),
            text.to_lowercase().chars().collect(),
        )
    } else {
        (pattern.chars().collect(), text.chars().collect())
    };
    glob_chars(&pc, &tc)
}

/// Drops a trailing ":port" from a host, if there is one.
fn strip_port(host: &str) -> &str {
    if let Some((h, port)) = host.rsplit_once(':') {
        if !port.is_empty()
            && port.chars().all(|c| c.is_ascii_digit())
            && !h.is_empty()
            && !h.ends_with(':')
        {
            return h;
        }
    }
    host
}

fn host_matches(pattern: &str, host: &str) -> bool {
    let host = host.trim();
    let host = if pattern.contains(':') {
        host
    } else {
        strip_port(host)
    };
    if glob(pattern, host, true) {
        return true;
    }
    // "*.example.com" also covers "example.com" itself.
    if let Some(apex) = pattern.strip_prefix("*.") {
        if !apex.contains('*') && apex.eq_ignore_ascii_case(host) {
            return true;
        }
    }
    false
}

fn path_matches(pattern: &str, path: &str) -> bool {
    let bare = path.split(['?', '#']).next().unwrap_or("");
    if pattern.contains('*') {
        glob(pattern, bare, false)
    } else {
        bare.starts_with(pattern)
    }
}

impl Matcher {
    fn has_condition(&self) -> bool {
        field(&self.host).is_some()
            || field(&self.path).is_some()
            || field(&self.method).is_some()
            || field(&self.agent).is_some()
    }

    /// All present fields must match. A matcher with no conditions matches nothing.
    pub fn matches(&self, agent: &str, method: &str, host: &str, path: &str) -> bool {
        if !self.has_condition() {
            return false;
        }
        if let Some(m) = field(&self.method) {
            if !m.eq_ignore_ascii_case(method.trim()) {
                return false;
            }
        }
        if let Some(h) = field(&self.host) {
            if !host_matches(h, host) {
                return false;
            }
        }
        if let Some(p) = field(&self.path) {
            if !path_matches(p, path) {
                return false;
            }
        }
        if let Some(a) = field(&self.agent) {
            if !glob(a, agent.trim(), true) {
                return false;
            }
        }
        true
    }

    /// Short label such as "POST *.stripe.com/v1/charges" or "agent shopper".
    pub fn describe(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(m) = field(&self.method) {
            parts.push(m.to_ascii_uppercase());
        }
        let target = format!(
            "{}{}",
            field(&self.host).unwrap_or(""),
            field(&self.path).unwrap_or("")
        );
        if !target.is_empty() {
            parts.push(target);
        }
        if let Some(a) = field(&self.agent) {
            parts.push(format!("agent {}", a));
        }
        if parts.is_empty() {
            "empty rule".to_string()
        } else {
            parts.join(" ")
        }
    }

    fn reason_text(&self) -> String {
        match field(&self.reason) {
            Some(r) => r.to_string(),
            None => format!("Matched rule: {}", self.describe()),
        }
    }
}

impl RulesFile {
    /// First matching [[block]] wins, then [[hold]], then [[allow]].
    pub fn verdict(&self, agent: &str, method: &str, host: &str, path: &str) -> Option<Verdict> {
        if let Some(m) = self.block.iter().find(|m| m.matches(agent, method, host, path)) {
            return Some(Verdict::Block(m.reason_text()));
        }
        if let Some(m) = self.hold.iter().find(|m| m.matches(agent, method, host, path)) {
            return Some(Verdict::Hold(m.reason_text()));
        }
        if self.allow.iter().any(|m| m.matches(agent, method, host, path)) {
            return Some(Verdict::Allow);
        }
        None
    }

    pub fn custom_rule_count(&self) -> usize {
        self.block.len() + self.hold.len() + self.allow.len()
    }
}

fn format_error(text: &str, err: &toml::de::Error) -> String {
    let msg = err.message().lines().next().unwrap_or("").trim().to_string();
    let msg = if msg.is_empty() {
        "this file could not be read".to_string()
    } else {
        msg
    };
    match err.span() {
        Some(span) => {
            let end = span.start.min(text.len());
            let line = text.as_bytes()[..end].iter().filter(|b| **b == b'\n').count() + 1;
            format!("line {}: {}", line, msg)
        }
        None => msg,
    }
}

/// Parses the rules file. Errors look like "line 3: unknown field `foo`, ...".
pub fn parse(text: &str) -> Result<RulesFile, String> {
    let only_comments = text.lines().all(|l| {
        let t = l.trim();
        t.is_empty() || t.starts_with('#')
    });
    if only_comments {
        return Ok(RulesFile::default());
    }
    toml::from_str::<RulesFile>(text).map_err(|e| format_error(text, &e))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Turns every commented-out setting in TEMPLATE back on.
    fn uncomment(template: &str) -> String {
        let mut out = String::new();
        for line in template.lines() {
            let mut keep = line.to_string();
            if let Some(rest) = line.strip_prefix("# ") {
                let looks_like_setting = rest.starts_with('[')
                    || match rest.split_once('=') {
                        Some((key, _)) => {
                            let k = key.trim();
                            !k.is_empty()
                                && k.chars().all(|c| {
                                    c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '"'
                                })
                        }
                        None => false,
                    };
                if looks_like_setting {
                    keep = rest.to_string();
                }
            }
            out.push_str(&keep);
            out.push('\n');
        }
        out
    }

    fn m(host: Option<&str>, path: Option<&str>, method: Option<&str>, agent: Option<&str>) -> Matcher {
        Matcher {
            host: host.map(String::from),
            path: path.map(String::from),
            method: method.map(String::from),
            agent: agent.map(String::from),
            reason: None,
        }
    }

    #[test]
    fn template_commented_is_default() {
        let r = parse(TEMPLATE).expect("template parses");
        assert!(r.budget.daily_ai_spend.is_none());
        assert!(r.rules.block_secrets.is_none());
        assert_eq!(r.custom_rule_count(), 0);
        assert!(r.agents.is_empty());
        assert!(r.pricing.is_empty());
        assert!(r.ai_endpoint.is_empty());
    }

    #[test]
    fn template_uncommented_parses() {
        let text = uncomment(TEMPLATE);
        let r = parse(&text).unwrap_or_else(|e| panic!("{}\n{}", e, text));
        assert_eq!(r.budget.daily_ai_spend, Some(10.0));
        assert_eq!(r.rules.hold_payments, Some(true));
        assert_eq!(r.rules.hold_checkout_pages, Some(true));
        assert_eq!(r.rules.block_secrets, Some(false));
        assert_eq!(r.rules.save_request_bodies, Some(true));
        assert_eq!(r.block.len(), 1);
        assert_eq!(r.hold.len(), 1);
        assert_eq!(r.allow.len(), 1);
        assert_eq!(r.custom_rule_count(), 3);
        assert_eq!(r.agents["researcher"].daily_ai_spend, Some(2.0));
        assert_eq!(r.ai_endpoint.len(), 1);
        assert!(r.is_ai_endpoint("llm.internal.example"));
        assert_eq!(r.pricing["my-finetune"].output, 15.0);
        assert_eq!(r.pricing["my-finetune"].cached_input, Some(0.3));
    }

    #[test]
    fn empty_text_is_default() {
        assert_eq!(parse("").unwrap().custom_rule_count(), 0);
        assert_eq!(parse("  \n# just a note\n").unwrap().custom_rule_count(), 0);
    }

    #[test]
    fn host_glob() {
        let x = m(Some("*.example.com"), None, None, None);
        assert!(x.matches("a", "GET", "api.example.com", "/"));
        assert!(x.matches("a", "GET", "API.Example.COM:443", "/"));
        assert!(x.matches("a", "GET", "example.com", "/"));
        assert!(!x.matches("a", "GET", "badexample.com", "/"));
        assert!(!x.matches("a", "GET", "example.org", "/"));
        let y = m(Some("api.*.com"), None, None, None);
        assert!(y.matches("a", "GET", "api.stripe.com", "/"));
        assert!(!y.matches("a", "GET", "api.stripe.org", "/"));
    }

    #[test]
    fn path_prefix_and_glob() {
        let prefix = m(None, Some("/v1/charges"), None, None);
        assert!(prefix.matches("a", "POST", "h", "/v1/charges"));
        assert!(prefix.matches("a", "POST", "h", "/v1/charges/ch_1"));
        assert!(prefix.matches("a", "POST", "h", "/v1/charges?x=1"));
        assert!(!prefix.matches("a", "POST", "h", "/v2/charges"));
        let g = m(None, Some("/v1/*/refund"), None, None);
        assert!(g.matches("a", "POST", "h", "/v1/ch_1/refund"));
        assert!(g.matches("a", "POST", "h", "/v1/ch_1/refund?y=2"));
        assert!(!g.matches("a", "POST", "h", "/v1/ch_1/refund/extra"));
    }

    #[test]
    fn method_and_agent() {
        let x = m(None, None, Some("post"), Some("shop*"));
        assert!(x.matches("Shopper", "POST", "h", "/"));
        assert!(!x.matches("Shopper", "GET", "h", "/"));
        assert!(!x.matches("coder", "POST", "h", "/"));
        assert_eq!(x.describe(), "POST agent shop*");
        assert_eq!(
            m(Some("*.stripe.com"), Some("/v1/charges"), Some("post"), None).describe(),
            "POST *.stripe.com/v1/charges"
        );
        assert_eq!(m(None, None, None, Some("shopper")).describe(), "agent shopper");
    }

    #[test]
    fn empty_matcher_matches_nothing() {
        let x = Matcher::default();
        assert!(!x.matches("a", "GET", "example.com", "/"));
        let blank = m(Some(""), Some("  "), None, None);
        assert!(!blank.matches("a", "GET", "example.com", "/"));
    }

    #[test]
    fn glob_is_not_exponential() {
        let pat = "*a*a*a*a*a*a*a*a*a*a*a*b";
        let text = "a".repeat(200);
        assert!(!glob(pat, &text, false));
    }

    #[test]
    fn verdict_precedence() {
        let text = r###"
[[allow]]
host = "*.stripe.com"

[[hold]]
host = "api.stripe.com"

[[block]]
host = "api.stripe.com"
path = "/v1/charges"
reason = "No charges"
"###;
        let r = parse(text).unwrap();
        assert_eq!(
            r.verdict("a", "POST", "api.stripe.com", "/v1/charges"),
            Some(Verdict::Block("No charges".to_string()))
        );
        assert_eq!(
            r.verdict("a", "GET", "api.stripe.com", "/v1/customers"),
            Some(Verdict::Hold("Matched rule: api.stripe.com".to_string()))
        );
        assert_eq!(
            r.verdict("a", "GET", "files.stripe.com", "/x"),
            Some(Verdict::Allow)
        );
        assert_eq!(r.verdict("a", "GET", "example.com", "/"), None);
    }

    #[test]
    fn unknown_key_reports_line() {
        let text = "[rules]\nhold_payments = true\nfoo = 1\n";
        let err = parse(text).unwrap_err();
        assert!(err.starts_with("line 3:"), "{}", err);
        assert!(err.contains("foo"), "{}", err);

        let err = parse("[budget]\ndaily_ai_spend = \"lots\"\n").unwrap_err();
        assert!(err.starts_with("line 2:"), "{}", err);
    }

    #[test]
    fn agents_and_pricing() {
        let text = r###"
[agents."research bot"]
daily_ai_spend = 1.5

[pricing."gpt-4o"]
input = 2.5
output = 10.0
"###;
        let r = parse(text).unwrap();
        assert_eq!(r.agents["research bot"].daily_ai_spend, Some(1.5));
        assert_eq!(r.pricing["gpt-4o"].input, 2.5);
        assert_eq!(r.pricing["gpt-4o"].cached_input, None);

        let err = parse("[pricing.x]\ninput = 1.0\n").unwrap_err();
        assert!(err.contains("output"), "{}", err);
    }
}
