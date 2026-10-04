import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isTauri } from "./tauri-ipc";

export type FlightLlm = {
  provider: string;
  model: string | null;
  inputTokens: number;
  outputTokens: number;
  cachedInputTokens: number;
  costUsd: number | null;
  estimated: boolean;
};

export type FlightEntry = {
  id: string;
  ts: number;
  agent: string;
  method: string;
  host: string;
  path: string;
  status: number | null;
  outcome: "passed" | "allowed" | "blocked" | "held" | "error";
  reason: string | null;
  durationMs: number | null;
  reqBytes: number;
  respBytes: number;
  threats: string[];
  llm: FlightLlm | null;
};

export type FlightDetail = FlightEntry & {
  url: string;
  requestHeaders: [string, string][];
  responseHeaders: [string, string][];
  requestBody: string | null;
  responseBody: string | null;
  bodiesSaved: boolean;
};

export type FlightQuery = { text?: string; agent?: string; llmOnly?: boolean; limit?: number };

export type SpendRow = {
  provider: string;
  model: string;
  calls: number;
  inputTokens: number;
  outputTokens: number;
  costUsd: number | null;
};

export type SpendAgentRow = { agent: string; calls: number; costUsd: number };

export type SpendSummary = {
  today: number;
  todayEstimated: boolean;
  cap: number | null;
  capReached: boolean;
  days: { date: string; costUsd: number }[];
  byModel: SpendRow[];
  byAgent: SpendAgentRow[];
  pricesAsOf: string;
  unpricedModels: string[];
  capFromRulesFile: boolean;
};

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

/* Flight recorder ------------------------------------------------------------ */

export const getFlight = (query: FlightQuery) => call<FlightEntry[]>("get_flight", { query });
export const getFlightEntry = (id: string) => call<FlightDetail>("get_flight_entry", { id });
export const clearFlight = () => call<boolean>("clear_flight");

export async function exportHar(
  query: FlightQuery,
): Promise<{ ok: boolean; path?: string; error?: string }> {
  if (!isTauri()) return { ok: false, error: "desktop app only" };
  try {
    return { ok: true, path: await invoke<string>("export_har", { query }) };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}

export const onFlightAdded = (cb: (e: FlightEntry) => void) => on<FlightEntry>("flight:added", cb);
export const onFlightUpdated = (cb: (e: FlightEntry) => void) =>
  on<FlightEntry>("flight:updated", cb);

/* AI spend ------------------------------------------------------------------- */

export const getSpend = () => call<SpendSummary>("get_spend");
export const setAiCap = (cap: number | null) => call<SpendSummary>("set_ai_cap", { cap });
export const onSpendChanged = (cb: (s: SpendSummary) => void) =>
  on<SpendSummary>("spend:changed", cb);
