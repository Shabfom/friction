//! Payment detection. Deterministic: a request is a payment when it hits a
//! known payment API endpoint, or (optionally) a checkout-style path on any
//! site. Amounts are read from each processor's documented request format.

pub struct Payment {
    /// "Stripe", "PayPal", ... or the host for checkout pages on other sites.
    pub processor: String,
    /// What the request does: "Payment", "Charge", "Subscription", ...
    pub action: &'static str,
    pub amount: Option<f64>,
    pub currency: Option<String>,
    /// True for the generic checkout-path match (not a known payment API).
    pub generic: bool,
}

impl Payment {
    pub fn amount_text(&self) -> Option<String> {
        let a = self.amount?;
        let cur = self.currency.clone().unwrap_or_default().to_uppercase();
        Some(match cur.as_str() {
            "USD" | "" => format!("${a:.2}"),
            "EUR" => format!("€{a:.2}"),
            "GBP" => format!("£{a:.2}"),
            "JPY" => format!("¥{a:.0}"),
            _ => format!("{a:.2} {cur}"),
        })
    }

    pub fn describe(&self) -> String {
        match self.amount_text() {
            Some(a) => format!("{} {} for {}", self.processor, self.action.to_lowercase(), a),
            None => format!("{} {}", self.processor, self.action.to_lowercase()),
        }
    }
}

/// (processor, host suffix, path prefix, action). Empty path matches any path.
const ENDPOINTS: &[(&str, &str, &str, &str)] = &[
    ("Stripe", "api.stripe.com", "/v1/payment_intents", "Payment"),
    ("Stripe", "api.stripe.com", "/v1/charges", "Charge"),
    ("Stripe", "api.stripe.com", "/v1/checkout/sessions", "Checkout"),
    ("Stripe", "api.stripe.com", "/v1/invoices", "Invoice"),
    ("Stripe", "api.stripe.com", "/v1/subscriptions", "Subscription"),
    ("Stripe", "api.stripe.com", "/v1/transfers", "Transfer"),
    ("Stripe", "api.stripe.com", "/v1/payouts", "Payout"),
    ("PayPal", "paypal.com", "/v2/checkout/orders", "Order"),
    ("PayPal", "paypal.com", "/v1/payments", "Payment"),
    ("PayPal", "paypal.com", "/v2/payments", "Payment"),
    ("PayPal", "paypal.com", "/v1/billing/subscriptions", "Subscription"),
    ("Square", "squareup.com", "/v2/payments", "Payment"),
    ("Square", "squareup.com", "/v2/subscriptions", "Subscription"),
    ("Square", "squareupsandbox.com", "/v2/payments", "Payment"),
    ("Adyen", "adyen.com", "", "Payment"),
    ("Adyen", "adyenpayments.com", "", "Payment"),
    ("Braintree", "braintree-api.com", "", "Transaction"),
    ("Braintree", "braintreegateway.com", "", "Transaction"),
    ("Shopify", "myshopify.com", "/checkouts", "Checkout"),
    ("Shopify", "checkout.shopify.com", "", "Checkout"),
    ("Coinbase", "api.commerce.coinbase.com", "/charges", "Crypto charge"),
    ("Coinbase", "api.coinbase.com", "/api/v3/brokerage/orders", "Trade"),
    ("Paddle", "api.paddle.com", "/transactions", "Transaction"),
    ("Lemon Squeezy", "api.lemonsqueezy.com", "/v1/checkouts", "Checkout"),
    ("Wise", "wise.com", "", "Transfer"),
    ("Wise", "transferwise.com", "", "Transfer"),
    ("Plaid", "plaid.com", "/transfer/", "Bank transfer"),
    ("Alpaca", "alpaca.markets", "/v2/orders", "Trade"),
];

/// Coinbase money movement under /v2/accounts/{id}/transactions.
fn coinbase_send(host: &str, path: &str) -> bool {
    host_matches(host, "api.coinbase.com") && path.starts_with("/v2/accounts/") && path.contains("/transactions")
}

const CHECKOUT_SEGMENTS: &[&str] = &[
    "checkout", "checkouts", "purchase", "purchases", "pay", "payment", "payments", "charge", "charges",
    "placeorder", "place-order", "place_order", "submitorder", "submit-order", "submit_order", "buy",
    "buy-now", "buynow",
];

fn host_matches(host: &str, suffix: &str) -> bool {
    let h = host.to_ascii_lowercase();
    h == suffix || h.ends_with(&format!(".{suffix}"))
}

/// Currencies whose API amounts have no minor unit.
const ZERO_DECIMAL: &[&str] =
    &["bif", "clp", "djf", "gnf", "jpy", "kmf", "krw", "mga", "pyg", "rwf", "ugx", "vnd", "vuv", "xaf", "xof", "xpf"];

fn from_minor(value: f64, currency: &str) -> f64 {
    if ZERO_DECIMAL.contains(&currency.to_ascii_lowercase().as_str()) {
        value
    } else {
        value / 100.0
    }
}

fn form_value(body: &str, key: &str) -> Option<String> {
    body.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| v.replace('+', " ").replace("%20", " "))
    })
}

fn num(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Reads the amount from the processor's documented request shape.
fn amount_for(processor: &str, body: &str, json: Option<&serde_json::Value>) -> (Option<f64>, Option<String>) {
    match processor {
        "Stripe" => {
            let cur = form_value(body, "currency");
            let amt = form_value(body, "amount").and_then(|a| a.parse::<f64>().ok());
            match (amt, cur) {
                (Some(a), Some(c)) => (Some(from_minor(a, &c)), Some(c)),
                (Some(a), None) => (Some(a / 100.0), None),
                _ => (None, None),
            }
        }
        "PayPal" => {
            let Some(units) = json.and_then(|j| j.get("purchase_units")).and_then(|u| u.as_array()) else {
                return (None, None);
            };
            let mut total = 0.0;
            let mut cur = None;
            for u in units {
                if let Some(a) = u.get("amount") {
                    total += a.get("value").and_then(num).unwrap_or(0.0);
                    cur = cur.or_else(|| a.get("currency_code").and_then(|c| c.as_str()).map(String::from));
                }
            }
            if total > 0.0 { (Some(total), cur) } else { (None, None) }
        }
        "Square" => {
            let m = json.and_then(|j| j.get("amount_money"));
            let cur = m.and_then(|m| m.get("currency")).and_then(|c| c.as_str()).map(String::from);
            let amt = m.and_then(|m| m.get("amount")).and_then(num);
            (amt.map(|a| from_minor(a, cur.as_deref().unwrap_or("usd"))), cur)
        }
        "Adyen" => {
            let m = json.and_then(|j| j.get("amount"));
            let cur = m.and_then(|m| m.get("currency")).and_then(|c| c.as_str()).map(String::from);
            let amt = m.and_then(|m| m.get("value")).and_then(num);
            (amt.map(|a| from_minor(a, cur.as_deref().unwrap_or("usd"))), cur)
        }
        "Coinbase" => {
            let m = json.and_then(|j| j.get("local_price").or_else(|| j.get("amount").filter(|a| a.is_object())));
            let cur = m.and_then(|m| m.get("currency")).and_then(|c| c.as_str()).map(String::from);
            (m.and_then(|m| m.get("amount")).and_then(num), cur)
        }
        _ => (None, None),
    }
}

/// Classifies a request. `include_checkout_pages` adds the generic path match.
pub fn classify(
    method: &str,
    host: &str,
    path: &str,
    body: &str,
    json: Option<&serde_json::Value>,
    include_checkout_pages: bool,
) -> Option<Payment> {
    if !matches!(method.to_ascii_uppercase().as_str(), "POST" | "PUT" | "PATCH") {
        return None;
    }
    let p = path.split(['?', '#']).next().unwrap_or("/");
    let hit = ENDPOINTS
        .iter()
        .find(|(_, h, prefix, _)| host_matches(host, h) && (prefix.is_empty() || p.starts_with(prefix)))
        .map(|(proc_, _, _, action)| (*proc_, *action))
        .or_else(|| coinbase_send(host, p).then_some(("Coinbase", "Crypto transfer")));
    if let Some((processor, action)) = hit {
        let (amount, currency) = amount_for(processor, body, json);
        return Some(Payment { processor: processor.into(), action, amount, currency, generic: false });
    }
    if include_checkout_pages && crate::spend::provider_for_host(host).is_none() {
        let is_checkout = p.split('/').any(|seg| {
            let s = seg.to_ascii_lowercase();
            CHECKOUT_SEGMENTS.contains(&s.as_str()) || s.starts_with("checkout")
        });
        if is_checkout {
            return Some(Payment {
                processor: host.to_string(),
                action: "Checkout",
                amount: None,
                currency: None,
                generic: true,
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stripe_payment_intent_with_amount() {
        let p = classify("POST", "api.stripe.com", "/v1/payment_intents", "amount=2000&currency=usd", None, true).unwrap();
        assert_eq!(p.processor, "Stripe");
        assert_eq!(p.amount_text().unwrap(), "$20.00");
        assert!(!p.generic);
    }

    #[test]
    fn stripe_reads_are_not_payments() {
        assert!(classify("GET", "api.stripe.com", "/v1/payment_intents", "", None, true).is_none());
        assert!(classify("POST", "api.stripe.com", "/v1/customers", "", None, true).is_none());
    }

    #[test]
    fn zero_decimal_currency() {
        let p = classify("POST", "api.stripe.com", "/v1/charges", "amount=500&currency=jpy", None, true).unwrap();
        assert_eq!(p.amount_text().unwrap(), "¥500");
    }

    #[test]
    fn paypal_sums_purchase_units() {
        let j: serde_json::Value = serde_json::from_str(
            r#"{"purchase_units":[{"amount":{"currency_code":"EUR","value":"10.50"}},{"amount":{"currency_code":"EUR","value":"4.50"}}]}"#,
        )
        .unwrap();
        let p = classify("POST", "api-m.paypal.com", "/v2/checkout/orders", "", Some(&j), true).unwrap();
        assert_eq!(p.amount_text().unwrap(), "€15.00");
    }

    #[test]
    fn square_and_adyen_minor_units() {
        let j: serde_json::Value = serde_json::from_str(r#"{"amount_money":{"amount":1999,"currency":"USD"}}"#).unwrap();
        let p = classify("POST", "connect.squareup.com", "/v2/payments", "", Some(&j), true).unwrap();
        assert_eq!(p.amount_text().unwrap(), "$19.99");
        let j: serde_json::Value = serde_json::from_str(r#"{"amount":{"value":4200,"currency":"GBP"}}"#).unwrap();
        let p = classify("POST", "checkout-test.adyen.com", "/v71/payments", "", Some(&j), true).unwrap();
        assert_eq!(p.amount_text().unwrap(), "£42.00");
    }

    #[test]
    fn generic_checkout_paths_are_optional() {
        assert!(classify("POST", "shop.example", "/cart/checkout", "", None, true).unwrap().generic);
        assert!(classify("POST", "shop.example", "/cart/checkout", "", None, false).is_none());
        assert!(classify("POST", "shop.example", "/api/payments/confirm", "", None, true).is_some());
        assert!(classify("POST", "shop.example", "/api/paypal-webhook-docs", "", None, true).is_none());
    }

    #[test]
    fn ai_api_calls_are_never_checkout_pages() {
        assert!(classify("POST", "api.openai.com", "/v1/responses/checkout", "", None, true).is_none());
    }

    #[test]
    fn lookalike_hosts_do_not_match() {
        assert!(classify("POST", "api.stripe.com.evil.example", "/v1/charges", "", None, false).is_none());
    }
}
