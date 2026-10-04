export type ThreatSeverity = "critical" | "warning" | "info";

export type Threat = {
  code: string;
  label: string;
  severity: ThreatSeverity;
  detail: string;
  /** which part of Friction raised this finding */
  source?: "rules" | "proxy" | undefined;
};

export type PayloadLine = {
  text: string;
  /** a line containing a masked secret */
  flagged?: boolean | undefined;
};

/** A request held for the user's decision (mirrors InterceptPayload in Rust). */
export type Intercept = {
  id: string;
  agent: string;
  /** payment processor name, or the host */
  destination: string;
  /** e.g. "Stripe payment for $20.00" or "POST /upload" */
  summary: string;
  /** display amount when Friction could read it, e.g. "$20.00" */
  amount: string | null;
  amountValue: number | null;
  timestamp: number;
  threats: Threat[];
  executionPayload: PayloadLine[];
  method: string;
  url: string;
  /** auto-block deadline (ms epoch) */
  expiresAt: number;
};

export type AuditOutcome = "approved" | "rejected";

export type AuditEntry = {
  id: string;
  interceptId: string;
  outcome: AuditOutcome;
  agent: string;
  /** destination (processor or host) */
  merchant: string;
  /** summary of the request */
  item: string;
  /** amount paid when allowed */
  amount: number;
  /** payment amount, when known */
  delta: number;
  threatsCaught: number;
  timestamp: number;
  /** true when Friction decided on its own (a rule, a limit or a timeout) */
  automatic?: boolean | undefined;
  reason?: string | undefined;
};

export type RuleKey =
  "holdPayments" | "holdCheckoutPages" | "blockSecretLeaks" | "logPayloadsLocally";

export type Rules = Record<RuleKey, boolean>;

/** Settings shared with the Rust proxy. */
export type FrictionConfig = {
  rules: Rules;
  seenTraffic: boolean;
  dailyAiCap: number | null;
  /** settings controlled by ~/.friction/rules.toml */
  fileOverrides: string[];
};

export const RULE_GROUPS: {
  group: string;
  rules: { key: RuleKey; label: string; detail: string }[];
}[] = [
  {
    group: "Payments",
    rules: [
      {
        key: "holdPayments",
        label: "Ask me before any payment",
        detail:
          "Holds requests to Stripe, PayPal, Square, Adyen, Braintree, Shopify checkout, Coinbase, Wise, Plaid and other payment APIs, with the amount when the request includes it.",
      },
      {
        key: "holdCheckoutPages",
        label: "Also catch checkout pages on other sites",
        detail: "Holds POSTs to checkout, payment and purchase paths on any website.",
      },
    ],
  },
  {
    group: "Secrets",
    rules: [
      {
        key: "blockSecretLeaks",
        label: "Block leaked secrets without asking",
        detail:
          "API keys, private keys and .env values headed anywhere but their own service are always caught. When off, they wait for you.",
      },
    ],
  },
  {
    group: "Privacy",
    rules: [
      {
        key: "logPayloadsLocally",
        label: "Save request bodies",
        detail:
          "Keeps request and response bodies for Activity and held requests in ~/.friction. Secrets are masked.",
      },
    ],
  },
];

export const DEFAULT_RULES: Rules = {
  holdPayments: true,
  holdCheckoutPages: true,
  blockSecretLeaks: false,
  logPayloadsLocally: true,
};

export function normalizeLine(line: PayloadLine | string): PayloadLine {
  return typeof line === "string" ? { text: line } : line;
}

export function formatCurrency(value: number): string {
  return new Intl.NumberFormat("en-US", { style: "currency", currency: "USD" }).format(value);
}

export function formatClock(ts: number): string {
  return new Date(ts).toLocaleTimeString("en-US", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}
