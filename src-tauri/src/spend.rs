//! Metering of AI API spend from proxied traffic.
//!
//! Pure module: no tauri, no I/O. The proxy feeds it the request (host, path,
//! body) and the first and last bytes of the response body, and gets back
//! token usage and an estimated cost in USD.

use serde_json::Value;

/// Date the built-in price table was last checked against the official pages.
pub const PRICES_AS_OF: &str = "2026-10-03";

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// Uncached input tokens.
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Input tokens served from the provider's prompt cache.
    pub cached_input_tokens: u64,
    /// Input tokens written to the provider's prompt cache.
    pub cache_write_tokens: u64,
    /// Cost the provider reported itself (OpenRouter), in USD.
    pub reported_cost_usd: Option<f64>,
    /// True when no usage block was found and the numbers are a rough guess.
    pub estimated: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmRequest {
    pub provider: &'static str,
    pub model: Option<String>,
    pub stream: bool,
}

/// USD per 1M tokens.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cached_input: Option<f64>,
    pub cache_write: Option<f64>,
}

// ---------------------------------------------------------------------------
// Providers
// ---------------------------------------------------------------------------

const PROVIDER_HOSTS: &[(&str, &str)] = &[
    ("api.openai.com", "OpenAI"),
    ("api.anthropic.com", "Anthropic"),
    ("openrouter.ai", "OpenRouter"),
    ("generativelanguage.googleapis.com", "Google"),
    ("api.groq.com", "Groq"),
    ("api.mistral.ai", "Mistral"),
    ("api.deepseek.com", "DeepSeek"),
    ("api.x.ai", "xAI"),
    ("api.together.xyz", "Together"),
    ("api.together.ai", "Together"),
    ("api.fireworks.ai", "Fireworks"),
    ("api.perplexity.ai", "Perplexity"),
    ("openai.azure.com", "Azure OpenAI"),
];

fn normalize_host(host: &str) -> String {
    let h = host.trim().to_ascii_lowercase();
    let without_port = match h.rsplit_once(':') {
        Some((name, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => h.as_str(),
    };
    without_port.trim_end_matches('.').to_string()
}

/// Provider name for a known AI API host. Matches the host itself or any
/// subdomain of it, never a look-alike such as "evilapi.openai.com.attacker".
pub fn provider_for_host(host: &str) -> Option<&'static str> {
    let h = normalize_host(host);
    for &(domain, name) in PROVIDER_HOSTS.iter() {
        if h == domain {
            return Some(name);
        }
        if h.len() > domain.len()
            && h.ends_with(domain)
            && h.as_bytes()[h.len() - domain.len() - 1] == b'.'
        {
            return Some(name);
        }
    }
    None
}

/// Path part of a request target, without query or fragment. Accepts both
/// origin-form ("/v1/x?y") and absolute-form ("https://host/v1/x").
fn clean_path(path: &str) -> &str {
    let mut p = path;
    if let Some(i) = p.find("://") {
        let after = &p[i + 3..];
        p = match after.find('/') {
            Some(j) => &after[j..],
            None => "/",
        };
    }
    let p = p.split(|c: char| c == '?' || c == '#').next().unwrap_or("");
    p.trim_end_matches('/')
}

/// Recognizes a text generation call. Returns None for anything else
/// (model listings, token counting, embeddings, files, and so on).
pub fn parse_request(host: &str, path: &str, body: &[u8]) -> Option<LlmRequest> {
    parse_request_as(provider_for_host(host)?, path, body)
}

/// Same as parse_request for a host the user declared as an AI endpoint
/// (an internal gateway, a self-hosted OpenAI-compatible server, and so on).
pub fn parse_request_as(provider: &'static str, path: &str, body: &[u8]) -> Option<LlmRequest> {
    let p = clean_path(path);

    if p.contains(":generateContent") || p.contains(":streamGenerateContent") {
        let model = p
            .find("/models/")
            .map(|i| p[i + "/models/".len()..].split(':').next().unwrap_or("").to_string())
            .filter(|m| !m.is_empty());
        let stream = p.contains(":streamGenerateContent");
        return Some(LlmRequest { provider, model, stream });
    }

    let is_generation = p.ends_with("/chat/completions")
        || p.ends_with("/completions")
        || p.ends_with("/responses")
        || p.ends_with("/v1/messages");
    if !is_generation {
        return None;
    }

    let json: Option<Value> = serde_json::from_slice(body).ok();
    let model = json
        .as_ref()
        .and_then(|v| v.get("model"))
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let stream = json
        .as_ref()
        .and_then(|v| v.get("stream"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some(LlmRequest { provider, model, stream })
}

// ---------------------------------------------------------------------------
// Response parsing
// ---------------------------------------------------------------------------

fn num(v: &Value, key: &str) -> Option<u64> {
    let x = v.get(key)?;
    if let Some(n) = x.as_u64() {
        return Some(n);
    }
    x.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64)
}

/// Parses one SSE "data:" line, or a bare JSON object line.
fn data_line_json(line: &str) -> Option<Value> {
    let t = line.trim();
    let t = match t.strip_prefix("data:") {
        Some(rest) => rest.trim_start(),
        None => t,
    };
    if !t.starts_with('{') {
        return None;
    }
    serde_json::from_str::<Value>(t).ok()
}

fn push_flat(out: &mut Vec<Value>, v: Value) {
    match v {
        Value::Array(items) => out.extend(items),
        other => out.push(other),
    }
}

fn collect_lines(text: &str, out: &mut Vec<Value>) {
    for line in text.lines() {
        if let Some(v) = data_line_json(line) {
            out.push(v);
        }
    }
}

/// Index one past the '}' that closes the object starting at `start`.
fn balanced_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth: i32 = 0;
    let mut in_str = false;
    let mut esc = false;
    for (j, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(j + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// Position right after `"key"` followed by optional spaces and a colon and
/// more optional spaces, for every occurrence of the key in `text`.
fn value_starts(text: &str, key: &str) -> Vec<usize> {
    let needle = format!("\"{}\"", key);
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = text[from..].find(&needle) {
        let mut i = from + rel + needle.len();
        from = i;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b':' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < bytes.len() {
            out.push(i);
        }
    }
    out
}

/// Every complete `"key": {...}` object in `text`, in order. Works on
/// fragments of a larger JSON document that would not parse as a whole.
fn extract_objects(text: &str, key: &str) -> Vec<Value> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    for i in value_starts(text, key) {
        if bytes[i] != b'{' {
            continue;
        }
        if let Some(end) = balanced_end(bytes, i) {
            if let Ok(v) = serde_json::from_str::<Value>(&text[i..end]) {
                out.push(v);
            }
        }
    }
    out
}

/// First `"key": "string"` value in `text` (no escape handling needed for ids).
fn first_string_value(text: &str, key: &str) -> Option<String> {
    let bytes = text.as_bytes();
    for i in value_starts(text, key) {
        if bytes[i] != b'"' {
            continue;
        }
        let rest = &text[i + 1..];
        if let Some(end) = rest.find(|c: char| c == '"' || c == '\\') {
            if rest.as_bytes()[end] == b'"' && end > 0 {
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}

fn apply_anthropic(us: &Value, u: &mut Usage) {
    if let Some(n) = num(us, "input_tokens") {
        u.input_tokens = n;
    }
    if let Some(n) = num(us, "output_tokens") {
        u.output_tokens = n;
    }
    if let Some(n) = num(us, "cache_read_input_tokens") {
        u.cached_input_tokens = n;
    }
    if let Some(n) = num(us, "cache_creation_input_tokens") {
        u.cache_write_tokens = n;
    }
}

fn apply_openai(us: &Value, u: &mut Usage) {
    if let Some(c) = us.get("cost").and_then(Value::as_f64) {
        u.reported_cost_usd = Some(c);
    }
    if let Some(prompt) = num(us, "prompt_tokens") {
        // Chat Completions: prompt_tokens includes cached tokens.
        let cached = us
            .get("prompt_tokens_details")
            .and_then(|d| num(d, "cached_tokens"))
            .or_else(|| num(us, "prompt_cache_hit_tokens"))
            .unwrap_or(0)
            .min(prompt);
        u.cached_input_tokens = cached;
        u.input_tokens = prompt - cached;
        if let Some(o) = num(us, "completion_tokens") {
            u.output_tokens = o;
        }
    } else if let Some(input) = num(us, "input_tokens") {
        // Responses API: input_tokens includes cached tokens.
        let cached = us
            .get("input_tokens_details")
            .and_then(|d| num(d, "cached_tokens"))
            .unwrap_or(0)
            .min(input);
        u.cached_input_tokens = cached;
        u.input_tokens = input - cached;
        if let Some(o) = num(us, "output_tokens") {
            u.output_tokens = o;
        }
    } else if let Some(o) = num(us, "completion_tokens").or_else(|| num(us, "output_tokens")) {
        u.output_tokens = o;
    }
}

fn apply_google(m: &Value, u: &mut Usage) {
    // promptTokenCount includes cached content.
    let prompt = num(m, "promptTokenCount").unwrap_or(0);
    let cached = num(m, "cachedContentTokenCount").unwrap_or(0).min(prompt);
    u.input_tokens = prompt - cached;
    u.cached_input_tokens = cached;
    // Thinking tokens are billed as output.
    u.output_tokens =
        num(m, "candidatesTokenCount").unwrap_or(0) + num(m, "thoughtsTokenCount").unwrap_or(0);
}

fn apply_usage(provider: &str, us: &Value, u: &mut Usage) {
    let anthropic_shape = us.get("cache_read_input_tokens").is_some()
        || us.get("cache_creation_input_tokens").is_some();
    if provider == "Anthropic" || anthropic_shape {
        apply_anthropic(us, u);
    } else {
        apply_openai(us, u);
    }
}

/// Folds one JSON value (a whole body or one stream event) into `u`.
/// Later values overwrite earlier ones, so the last usage seen wins.
fn apply_value(provider: &str, v: &Value, u: &mut Usage) -> bool {
    let mut found = false;
    if let Some(m) = v.get("usageMetadata") {
        if m.is_object() {
            apply_google(m, u);
            found = true;
        }
    }
    if v.get("type").and_then(Value::as_str) == Some("message_start") {
        if let Some(us) = v.get("message").and_then(|m| m.get("usage")) {
            if us.is_object() {
                apply_anthropic(us, u);
                found = true;
            }
        }
    }
    let nested = [
        v.get("usage"),
        v.get("response").and_then(|r| r.get("usage")),
        v.get("x_groq").and_then(|r| r.get("usage")),
    ];
    for us in nested.iter().flatten() {
        if us.is_object() {
            apply_usage(provider, us, u);
            found = true;
        }
    }
    found
}

/// Token usage from a response body. `head` is the start of the body, `tail`
/// the end of it (empty when the whole body fit in `head`).
pub fn parse_usage(
    provider: &str,
    head: &[u8],
    tail: &[u8],
    total_response_bytes: u64,
    request_body_len: usize,
) -> Usage {
    let head_s = String::from_utf8_lossy(head);
    let tail_s = String::from_utf8_lossy(tail);

    let mut values: Vec<Value> = Vec::new();
    if tail.is_empty() {
        if let Ok(v) = serde_json::from_str::<Value>(head_s.trim()) {
            push_flat(&mut values, v);
        }
    }
    collect_lines(&head_s, &mut values);
    if !tail.is_empty() {
        collect_lines(&tail_s, &mut values);
    }

    let mut usage = Usage::default();
    let mut found = false;
    for v in &values {
        if apply_value(provider, v, &mut usage) {
            found = true;
        }
    }

    if !found {
        // Large or truncated bodies: pull complete usage objects out of the
        // fragments directly.
        let texts: [&str; 2] = [&head_s, &tail_s];
        for text in texts.iter() {
            for key in ["usage", "usageMetadata"].iter() {
                for obj in extract_objects(text, key) {
                    let mut m = serde_json::Map::new();
                    m.insert((*key).to_string(), obj);
                    if apply_value(provider, &Value::Object(m), &mut usage) {
                        found = true;
                    }
                }
            }
        }
    }

    if !found {
        return Usage {
            input_tokens: request_body_len as u64 / 4,
            output_tokens: total_response_bytes / 4,
            estimated: true,
            ..Usage::default()
        };
    }
    usage
}

fn model_in(v: &Value) -> Option<String> {
    let candidates = [
        v.get("model"),
        v.get("message").and_then(|m| m.get("model")),
        v.get("response").and_then(|r| r.get("model")),
        v.get("modelVersion"),
    ];
    candidates
        .iter()
        .flatten()
        .filter_map(|x| x.as_str())
        .find(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// First model id named in a response body (JSON or SSE).
pub fn model_from_response(head: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(head);
    if let Ok(v) = serde_json::from_str::<Value>(s.trim()) {
        if let Some(m) = model_in(&v) {
            return Some(m);
        }
        if let Some(items) = v.as_array() {
            for item in items {
                if let Some(m) = model_in(item) {
                    return Some(m);
                }
            }
        }
    }
    for line in s.lines() {
        if let Some(v) = data_line_json(line) {
            if let Some(m) = model_in(&v) {
                return Some(m);
            }
        }
    }
    first_string_value(&s, "model").or_else(|| first_string_value(&s, "modelVersion"))
}

// ---------------------------------------------------------------------------
// Prices (USD per 1M tokens, standard tier, short context)
// ---------------------------------------------------------------------------

const fn p(input: f64, cached: f64, output: f64) -> Price {
    Price { input, output, cached_input: Some(cached), cache_write: None }
}

const fn pw(input: f64, cached: f64, write: f64, output: f64) -> Price {
    Price { input, output, cached_input: Some(cached), cache_write: Some(write) }
}

const PRICES: &[(&str, Price)] = &[
    // OpenAI. Source: https://developers.openai.com/api/docs/pricing
    ("gpt-6-astra", pw(5.00, 0.50, 6.25, 25.00)),
    ("gpt-6.1-sol", pw(1.00, 0.05, 1.25, 5.00)),
    ("gpt-6-luna", pw(0.05, 0.005, 0.0625, 0.25)),
    ("chat-latest", p(5.00, 0.50, 30.00)),
    ("gpt-5.3-codex", p(1.75, 0.175, 14.00)),
    ("gpt-rosalind-research", p(5.00, 0.50, 25.00)),
    // Anthropic. Cache write is the 5 minute rate.
    // Source: https://platform.claude.com/docs/en/about-claude/pricing
    ("claude-fable-5-1", pw(10.00, 0.25, 12.50, 50.00)),
    ("claude-mythos-5-1", pw(10.00, 0.25, 12.50, 50.00)),
    ("claude-fable-5", pw(10.00, 1.00, 12.50, 50.00)),
    ("claude-mythos-5", pw(10.00, 1.00, 12.50, 50.00)),
    ("claude-opus-5-5", pw(4.00, 0.20, 5.00, 20.00)),
    ("claude-opus-5", pw(5.00, 0.50, 6.25, 25.00)),
    ("claude-opus-4-8", pw(5.00, 0.50, 6.25, 25.00)),
    ("claude-opus-4-7", pw(5.00, 0.50, 6.25, 25.00)),
    ("claude-opus-4-6", pw(5.00, 0.50, 6.25, 25.00)),
    ("claude-opus-4-5", pw(5.00, 0.50, 6.25, 25.00)),
    ("claude-opus-4-1", pw(15.00, 1.50, 18.75, 75.00)),
    ("claude-opus-4-0", pw(15.00, 1.50, 18.75, 75.00)),
    ("claude-opus-4", pw(15.00, 1.50, 18.75, 75.00)),
    ("claude-sonnet-5-5", pw(2.00, 0.20, 2.50, 10.00)),
    ("claude-sonnet-5", pw(2.00, 0.20, 2.50, 10.00)),
    ("claude-sonnet-4-6", pw(3.00, 0.30, 3.75, 15.00)),
    ("claude-sonnet-4-5", pw(3.00, 0.30, 3.75, 15.00)),
    ("claude-sonnet-4-0", pw(3.00, 0.30, 3.75, 15.00)),
    ("claude-sonnet-4", pw(3.00, 0.30, 3.75, 15.00)),
    ("claude-haiku-4-5", pw(1.00, 0.10, 1.25, 5.00)),
    ("claude-3-5-haiku", pw(0.80, 0.08, 1.00, 4.00)),
    // Google. Cached = context caching rate (storage not counted). Pro is the
    // up to 200k prompt tier. Source: https://ai.google.dev/gemini-api/docs/pricing
    ("gemini-3.8-flash", p(0.75, 0.075, 3.75)),
    ("gemini-3.7-flash", p(0.75, 0.075, 3.75)),
    ("gemini-3.6-flash", p(0.75, 0.075, 3.75)),
    ("gemini-3.5-flash", p(1.50, 0.15, 9.00)),
    ("gemini-3.5-flash-lite", p(0.30, 0.03, 2.50)),
    ("gemini-3.1-flash-lite", p(0.25, 0.025, 1.50)),
    ("gemini-3.1-pro-preview", p(2.00, 0.20, 12.00)),
    ("gemini-3-flash-preview", p(0.50, 0.05, 3.00)),
    // DeepSeek. Peak hour rates (off peak is half), so this is an upper bound.
    // Source: https://api-docs.deepseek.com/quick_start/pricing
    ("deepseek-flash", p(0.30, 0.006, 1.20)),
    ("deepseek-v4-pro", p(1.32, 0.044, 3.96)),
];

/// Strips a trailing -YYYYMMDD or -YYYY-MM-DD snapshot date.
fn strip_date(m: &str) -> &str {
    let b = m.as_bytes();
    let n = b.len();
    let digits = |s: &[u8]| s.iter().all(u8::is_ascii_digit);
    if n > 9 && b[n - 9] == b'-' && digits(&b[n - 8..]) {
        return &m[..n - 9];
    }
    if n > 11
        && b[n - 11] == b'-'
        && digits(&b[n - 10..n - 6])
        && b[n - 6] == b'-'
        && digits(&b[n - 5..n - 3])
        && b[n - 3] == b'-'
        && digits(&b[n - 2..])
    {
        return &m[..n - 11];
    }
    m
}

fn normalize_model(model: &str) -> String {
    let lower = model.trim().to_ascii_lowercase();
    let last = match lower.rfind('/') {
        Some(i) => &lower[i + 1..],
        None => lower.as_str(),
    };
    strip_date(last).to_string()
}

/// True when `model` is `key` or `key` plus a variant suffix. A suffix that
/// looks like a newer version ("-9", "-4.1") or a different tier ("-lite",
/// "-mini") does not match, so it never borrows the wrong price.
fn prefix_matches(model: &str, key: &str) -> bool {
    let rest = match model.strip_prefix(key) {
        Some(r) => r,
        None => return false,
    };
    let first = match rest.chars().next() {
        Some(c) => c,
        None => return true,
    };
    if !matches!(first, '-' | ':' | '@') {
        return false;
    }
    let seg = rest[1..]
        .split(|c: char| c == '-' || c == ':' || c == '@')
        .next()
        .unwrap_or("");
    let starts_digit = seg.as_bytes().first().map_or(false, |b| b.is_ascii_digit());
    if seg.len() == 1 && starts_digit {
        return false;
    }
    if starts_digit && seg.contains('.') {
        return false;
    }
    !matches!(seg, "lite" | "mini" | "nano" | "pro" | "tts" | "image")
}

/// Built-in list price for a model id, after normalizing provider prefixes
/// ("anthropic/...") and snapshot dates. Longest matching table key wins.
pub fn builtin_price(model: &str) -> Option<Price> {
    let norm = normalize_model(model);
    let mut best: Option<(usize, Price)> = None;
    for &(key, price) in PRICES.iter() {
        if prefix_matches(&norm, key) && best.map_or(true, |(len, _)| key.len() > len) {
            best = Some((key.len(), price));
        }
    }
    best.map(|(_, price)| price)
}

/// Cost in USD. A provider reported cost wins; otherwise a price is needed.
pub fn cost_usd(usage: &Usage, price: Option<&Price>) -> Option<f64> {
    if let Some(c) = usage.reported_cost_usd {
        return Some(c);
    }
    let p = price?;
    let cached = p.cached_input.unwrap_or(p.input);
    let write = p.cache_write.unwrap_or(p.input * 1.25);
    let total = usage.input_tokens as f64 * p.input
        + usage.output_tokens as f64 * p.output
        + usage.cached_input_tokens as f64 * cached
        + usage.cache_write_tokens as f64 * write;
    Some(total / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn hosts() {
        assert_eq!(provider_for_host("api.openai.com"), Some("OpenAI"));
        assert_eq!(provider_for_host("API.Anthropic.com:443"), Some("Anthropic"));
        assert_eq!(provider_for_host("eu.openrouter.ai"), Some("OpenRouter"));
        assert_eq!(provider_for_host("notopenrouter.ai"), None);
        assert_eq!(provider_for_host("api.openai.com.evil.net"), None);
    }

    #[test]
    fn requests() {
        let r = parse_request(
            "api.openai.com",
            "/v1/chat/completions?x=1",
            br#"{"model":"gpt-6-luna","stream":true}"#,
        )
        .unwrap();
        assert_eq!(r.provider, "OpenAI");
        assert_eq!(r.model.as_deref(), Some("gpt-6-luna"));
        assert!(r.stream);
        assert!(parse_request("api.anthropic.com", "/v1/messages/count_tokens", b"{}").is_none());
        assert!(parse_request("api.openai.com", "/v1/embeddings", b"{}").is_none());
        assert!(parse_request("example.com", "/v1/chat/completions", b"{}").is_none());
        let g = parse_request(
            "generativelanguage.googleapis.com",
            "/v1beta/models/gemini-3.5-flash:streamGenerateContent?alt=sse",
            b"{}",
        )
        .unwrap();
        assert_eq!(g.model.as_deref(), Some("gemini-3.5-flash"));
        assert!(g.stream);
    }

    #[test]
    fn openai_chat_json() {
        let body = br#"{"id":"x","model":"gpt-6.1-sol","choices":[],"usage":{"prompt_tokens":1000,"completion_tokens":200,"prompt_tokens_details":{"cached_tokens":400}}}"#;
        let u = parse_usage("OpenAI", body, b"", body.len() as u64, 50);
        assert!(!u.estimated);
        assert_eq!(u.input_tokens, 600);
        assert_eq!(u.cached_input_tokens, 400);
        assert_eq!(u.output_tokens, 200);
        assert_eq!(model_from_response(body).as_deref(), Some("gpt-6.1-sol"));
    }

    #[test]
    fn openai_chat_sse_usage_in_last_chunk() {
        let body = b"data: {\"model\":\"gpt-6-luna\",\"choices\":[{\"delta\":{\"content\":\"hi\"}}],\"usage\":null}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":50,\"completion_tokens\":7}}\n\ndata: [DONE]\n\n";
        let u = parse_usage("OpenAI", body, b"", body.len() as u64, 10);
        assert!(!u.estimated);
        assert_eq!(u.input_tokens, 50);
        assert_eq!(u.output_tokens, 7);
        assert_eq!(model_from_response(body).as_deref(), Some("gpt-6-luna"));
    }

    const RESPONSES_SSE: &str = "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"model\":\"gpt-6-luna\",\"usage\":null}}\n\nevent: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"model\":\"gpt-6-luna\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"hello there\"}]}],\"usage\":{\"input_tokens\":300,\"input_tokens_details\":{\"cached_tokens\":100},\"output_tokens\":40}}}\n\n";

    #[test]
    fn responses_api_sse() {
        let body = RESPONSES_SSE.as_bytes();
        let u = parse_usage("OpenAI", body, b"", body.len() as u64, 10);
        assert!(!u.estimated);
        assert_eq!(u.input_tokens, 200);
        assert_eq!(u.cached_input_tokens, 100);
        assert_eq!(u.output_tokens, 40);
    }

    #[test]
    fn responses_api_sse_split_mid_event() {
        let body = RESPONSES_SSE.as_bytes();
        let cut = RESPONSES_SSE.find("hello").unwrap();
        let head = &body[..cut];
        let tail = &body[cut..];
        let u = parse_usage("OpenAI", head, tail, body.len() as u64, 10);
        assert!(!u.estimated);
        assert_eq!(u.input_tokens, 200);
        assert_eq!(u.output_tokens, 40);
        // Truncated in the middle of an object: must not panic.
        let _ = parse_usage("OpenAI", &body[..cut + 3], b"", 0, 0);
    }

    #[test]
    fn anthropic_sse() {
        let body = b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-opus-5-5\",\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":2000,\"cache_creation_input_tokens\":500,\"output_tokens\":1}}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":321}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
        let u = parse_usage("Anthropic", body, b"", body.len() as u64, 10);
        assert!(!u.estimated);
        assert_eq!(u.input_tokens, 10);
        assert_eq!(u.cached_input_tokens, 2000);
        assert_eq!(u.cache_write_tokens, 500);
        assert_eq!(u.output_tokens, 321);
        assert_eq!(model_from_response(body).as_deref(), Some("claude-opus-5-5"));
    }

    #[test]
    fn openrouter_cost() {
        let body = br#"{"model":"anthropic/claude-sonnet-5","usage":{"prompt_tokens":100,"completion_tokens":20,"cost":0.0123}}"#;
        let u = parse_usage("OpenRouter", body, b"", body.len() as u64, 10);
        assert_eq!(u.reported_cost_usd, Some(0.0123));
        assert_eq!(cost_usd(&u, None), Some(0.0123));
        let price = builtin_price("anthropic/claude-sonnet-5").unwrap();
        assert_eq!(cost_usd(&u, Some(&price)), Some(0.0123));
    }

    #[test]
    fn google_json() {
        let body = br#"{"candidates":[],"usageMetadata":{"promptTokenCount":120,"candidatesTokenCount":30,"cachedContentTokenCount":20},"modelVersion":"gemini-3.5-flash"}"#;
        let u = parse_usage("Google", body, b"", body.len() as u64, 10);
        assert!(!u.estimated);
        assert_eq!(u.input_tokens, 100);
        assert_eq!(u.cached_input_tokens, 20);
        assert_eq!(u.output_tokens, 30);
        assert_eq!(model_from_response(body).as_deref(), Some("gemini-3.5-flash"));
    }

    #[test]
    fn google_json_array_stream_truncated() {
        let head = b"[{\"candidates\":[],\"usageMetadata\":{\"promptTokenCount\":5}},\n{\"candidates\":[{\"content\":";
        let tail = b"\"x\"}],\"usageMetadata\":{\"promptTokenCount\":5,\"candidatesTokenCount\":9}}]";
        let u = parse_usage("Google", head, tail, 1000, 10);
        assert!(!u.estimated);
        assert_eq!(u.input_tokens, 5);
        assert_eq!(u.output_tokens, 9);
    }

    #[test]
    fn estimate_fallback() {
        let u = parse_usage("OpenAI", b"<html>bad gateway</html>", b"", 4000, 800);
        assert!(u.estimated);
        assert_eq!(u.input_tokens, 200);
        assert_eq!(u.output_tokens, 1000);
        assert_eq!(u.cached_input_tokens, 0);
    }

    #[test]
    fn price_normalization() {
        let sonnet = builtin_price("claude-sonnet-4-5").unwrap();
        assert_eq!(builtin_price("anthropic/claude-sonnet-4-5-20250929"), Some(sonnet));
        assert_eq!(builtin_price("claude-opus-4-20250514").unwrap().input, 15.0);
        assert_eq!(builtin_price("claude-opus-4-5-20251101").unwrap().input, 5.0);
        assert_eq!(builtin_price("openai/GPT-6-Luna-2026-08-01").unwrap().output, 0.25);
        assert_eq!(builtin_price("claude-haiku-4-5-20251001").unwrap().input, 1.0);
        assert_eq!(builtin_price("gemini-3.5-flash-lite").unwrap().input, 0.30);
        assert_eq!(builtin_price("models/gemini-3.5-flash").unwrap().input, 1.50);
        assert_eq!(builtin_price("claude-3-5-haiku-latest").unwrap().input, 0.80);
        // Unknown newer versions and tiers do not borrow a price.
        assert_eq!(builtin_price("claude-opus-4-9"), None);
        assert_eq!(builtin_price("gemini-3.8-flash-lite"), None);
        assert_eq!(builtin_price("some-local-model"), None);
    }

    #[test]
    fn cost_math_with_cache() {
        let u = Usage {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            cached_input_tokens: 1_000_000,
            cache_write_tokens: 1_000_000,
            ..Usage::default()
        };
        let opus = builtin_price("claude-opus-5-5").unwrap();
        assert!(close(cost_usd(&u, Some(&opus)).unwrap(), 4.0 + 20.0 + 0.2 + 5.0));
        let bare = Price { input: 2.0, output: 10.0, cached_input: None, cache_write: None };
        assert!(close(cost_usd(&u, Some(&bare)).unwrap(), 2.0 + 10.0 + 2.0 + 2.5));
        assert_eq!(cost_usd(&u, None), None);
    }
}
