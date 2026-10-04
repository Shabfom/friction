import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import {
  DEFAULT_RULES,
  type AuditEntry,
  type AuditOutcome,
  type FrictionConfig,
  type Intercept,
  type RuleKey,
  type Rules,
} from "./friction-data";
import {
  clearAuditLog,
  clearSpend,
  getAuditLog,
  getConfig,
  getPendingIntercepts,
  getProxyStatus,
  isTauri,
  onAuditAdded,
  onConfigChanged,
  onIntercept,
  onInterceptExpired,
  onProxyStatus,
  resolveInterceptNative,
  updateConfig,
  type ProxyStatus,
} from "./tauri-ipc";
import { clearFlight } from "./activity-ipc";

/*
 * The Rust proxy owns policy, held requests and history (~/.friction). This
 * hook mirrors that state and sends the user's actions back. In a plain
 * browser (the Vite preview) there is no proxy, so everything stays empty.
 */
export function useFriction() {
  const desktop = isTauri();
  const [intercepts, setIntercepts] = useState<Intercept[]>([]);
  const [audit, setAudit] = useState<AuditEntry[]>([]);
  const [rules, setRules] = useState<Rules>(DEFAULT_RULES);
  const [seenTraffic, setSeenTraffic] = useState(false);
  const [fileOverrides, setFileOverrides] = useState<string[]>([]);
  const [proxyStatus, setProxyStatus] = useState<ProxyStatus | null>(null);
  const [ready, setReady] = useState(!desktop);

  const interceptsRef = useRef<Intercept[]>([]);
  const rulesRef = useRef<Rules>(DEFAULT_RULES);

  const applyIntercepts = useCallback((next: Intercept[]) => {
    interceptsRef.current = next;
    setIntercepts(next);
  }, []);

  const applyConfig = useCallback((c: FrictionConfig) => {
    const r = { ...DEFAULT_RULES, ...c.rules };
    rulesRef.current = r;
    setRules(r);
    setSeenTraffic(!!c.seenTraffic);
    setFileOverrides(Array.isArray(c.fileOverrides) ? c.fileOverrides : []);
  }, []);

  useEffect(() => {
    if (!desktop) return;
    let cancelled = false;
    const unlisteners: Promise<() => void>[] = [
      onIntercept((incoming) => {
        applyIntercepts([incoming, ...interceptsRef.current.filter((i) => i.id !== incoming.id)]);
      }),
      onInterceptExpired((id) => {
        const target = interceptsRef.current.find((i) => i.id === id);
        applyIntercepts(interceptsRef.current.filter((i) => i.id !== id));
        if (target) toast(`${target.destination}: closed before you decided, so it was blocked.`);
      }),
      onAuditAdded((entry) => {
        setAudit((prev) => [entry, ...prev.filter((e) => e.id !== entry.id)]);
      }),
      onConfigChanged(applyConfig),
      onProxyStatus(setProxyStatus),
    ];

    void Promise.all([getConfig(), getPendingIntercepts(), getAuditLog(), getProxyStatus()]).then(
      ([config, pending, log, status]) => {
        if (cancelled) return;
        if (config) applyConfig(config);
        if (pending) {
          const ids = new Set(pending.map((p) => p.id));
          applyIntercepts([...pending, ...interceptsRef.current.filter((i) => !ids.has(i.id))]);
        }
        if (log) setAudit(log);
        if (status) setProxyStatus(status);
        setReady(true);
      },
    );

    return () => {
      cancelled = true;
      unlisteners.forEach((u) => void u.then((fn) => fn()));
    };
  }, [desktop, applyConfig, applyIntercepts]);

  const resolve = useCallback(
    (id: string, outcome: AuditOutcome) => {
      if (!interceptsRef.current.some((i) => i.id === id)) return;
      applyIntercepts(interceptsRef.current.filter((i) => i.id !== id));
      // Rust forwards or blocks the request and writes the history entry.
      void resolveInterceptNative(id, outcome === "approved" ? "approve" : "reject").then((ok) => {
        if (!ok) toast.error("Too late: that request had already been blocked.");
      });
    },
    [applyIntercepts],
  );

  const approve = useCallback((id: string) => resolve(id, "approved"), [resolve]);
  const reject = useCallback((id: string) => resolve(id, "rejected"), [resolve]);

  const toggleRule = useCallback((key: RuleKey) => {
    const next = { ...rulesRef.current, [key]: !rulesRef.current[key] };
    rulesRef.current = next;
    setRules(next);
    void updateConfig({ rules: next });
  }, []);

  /** Clears decisions, activity and AI spend history on this Mac. Rules are kept. */
  const clearHistory = useCallback(async () => {
    setAudit([]);
    await Promise.all([clearAuditLog(), clearFlight(), clearSpend()]);
  }, []);

  const restoreDefaultRules = useCallback(() => {
    rulesRef.current = DEFAULT_RULES;
    setRules(DEFAULT_RULES);
    void updateConfig({ rules: DEFAULT_RULES });
  }, []);

  return {
    desktop,
    ready,
    intercepts,
    audit,
    rules,
    seenTraffic,
    fileOverrides,
    proxyStatus,
    approve,
    reject,
    toggleRule,
    clearHistory,
    restoreDefaultRules,
  };
}
