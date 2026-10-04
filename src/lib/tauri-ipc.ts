import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AuditEntry, FrictionConfig, Intercept, Rules } from "./friction-data";

export type ProxyStatus = {
  kind: "ok" | "port_in_use" | "bind_failed";
  addr: string;
  message: string;
};

export type CaInfo = {
  available: boolean;
  path: string | null;
  trusted: boolean;
  error: string | null;
};

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function on<T>(event: string, cb: (payload: T) => void): Promise<() => void> {
  if (!isTauri()) return () => {};
  return listen<T>(event, (e) => cb(e.payload));
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<T>(cmd, args);
  } catch (err) {
    console.error(`[Friction IPC] ${cmd} failed:`, err);
    return null;
  }
}

/* Events pushed by the Rust proxy -------------------------------------------- */

export const onIntercept = (cb: (i: Intercept) => void) => on<Intercept>("intercept:detected", cb);
export const onInterceptExpired = (cb: (id: string) => void) => on<string>("intercept:expired", cb);
export const onAuditAdded = (cb: (e: AuditEntry) => void) => on<AuditEntry>("audit:added", cb);
export const onConfigChanged = (cb: (c: FrictionConfig) => void) =>
  on<FrictionConfig>("config:changed", cb);
export const onProxyStatus = (cb: (s: ProxyStatus) => void) => on<ProxyStatus>("proxy:status", cb);

/* Commands ------------------------------------------------------------------- */

export const getPendingIntercepts = () => call<Intercept[]>("get_pending_intercepts");
export const getProxyStatus = () => call<ProxyStatus | null>("get_proxy_status");
export const getConfig = () => call<FrictionConfig>("get_config");
export const updateConfig = (patch: { rules?: Rules }) =>
  call<FrictionConfig>("update_config", { patch });
export const getAuditLog = () => call<AuditEntry[]>("get_audit_log");
export const clearAuditLog = () => call<boolean>("clear_audit_log");
export const getCaInfo = () => call<CaInfo>("get_ca_info");
export const revealCa = () => call<boolean>("reveal_ca");

export async function installCa(): Promise<{ ok: boolean; error?: string }> {
  if (!isTauri()) return { ok: false, error: "only available in the desktop app" };
  try {
    const trusted = await invoke<boolean>("install_ca");
    return trusted
      ? { ok: true }
      : { ok: false, error: "macOS did not mark the certificate as trusted" };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}

export async function resolveInterceptNative(
  id: string,
  decision: "approve" | "reject",
): Promise<boolean> {
  if (!isTauri()) return true;
  return (await call<boolean>("resolve_intercept", { id, decision })) ?? false;
}

/* App shell (menu bar, app menu, login item) --------------------------------- */

export const onNavigate = (cb: (view: string) => void) => on<string>("navigate", cb);
export const onAutostartChanged = (cb: (enabled: boolean) => void) =>
  on<boolean>("autostart:changed", cb);
export const getAutostart = () => call<boolean>("get_autostart");
export const setAutostart = (enabled: boolean) => call<boolean>("set_autostart", { enabled });

/* Live counters and the demo request ---------------------------------------- */

export type Stats = { inspected: number; held: number; blocked: number };
export const getStats = () => call<Stats>("get_stats");
export type DemoKind = "purchase" | "leak";
export const sendDemoRequest = (kind: DemoKind = "purchase") =>
  call<null>("send_demo_request", { kind });

export const cliStatus = () => call<{ installedAt: string | null }>("cli_status");
export async function installCli(): Promise<{ ok: boolean; path?: string; error?: string }> {
  if (!isTauri()) return { ok: false, error: "desktop app only" };
  try {
    return { ok: true, path: await invoke<string>("install_cli") };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}

/* rules.toml ------------------------------------------------------------------ */

export type RulesFileStatus = {
  path: string;
  ok: boolean;
  error: string | null;
  customRules: number;
  overrides: string[];
};
export const getRulesFileStatus = () => call<RulesFileStatus>("get_rules_file_status");
export const openRulesFile = () => call<boolean>("open_rules_file");
export const revealRulesFile = () => call<boolean>("reveal_rules_file");
export const onRulesChanged = (cb: (s: RulesFileStatus) => void) =>
  on<RulesFileStatus>("rules:changed", cb);

export const clearSpend = () => call<boolean>("clear_spend");
