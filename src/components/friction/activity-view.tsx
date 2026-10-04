import { useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "@/components/ui/sheet";
import { formatClock } from "@/lib/friction-data";
import { isTauri } from "@/lib/tauri-ipc";
import {
  clearFlight,
  exportHar,
  getFlight,
  getFlightEntry,
  onFlightAdded,
  onFlightUpdated,
  type FlightDetail,
  type FlightEntry,
  type FlightQuery,
} from "@/lib/activity-ipc";
import { cn } from "@/lib/utils";

const MAX_KEPT = 2000;
const MAX_SHOWN = 500;
const BODY_LIMIT = 200_000;

function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function formatDuration(ms: number | null): string {
  if (ms == null) return "";
  if (ms < 1000) return `${Math.round(ms)} ms`;
  return `${(ms / 1000).toFixed(ms < 10_000 ? 2 : 1)} s`;
}

function formatUsd(v: number): string {
  if (v === 0) return "$0.00";
  if (Math.abs(v) < 1) return `$${v.toFixed(4)}`;
  return `$${v.toFixed(2)}`;
}

function llmCost(e: FlightEntry): string {
  if (!e.llm || e.llm.costUsd == null) return "";
  return `${e.llm.estimated ? "≈" : ""}${formatUsd(e.llm.costUsd)}`;
}

function statusClass(status: number | null): string {
  if (status == null) return "text-muted-foreground";
  if (status >= 500) return "text-threat";
  if (status >= 400) return "text-warn";
  return "text-muted-foreground";
}

const OUTCOME_LABEL: Record<FlightEntry["outcome"], string> = {
  passed: "",
  allowed: "Allowed",
  blocked: "Blocked",
  held: "Held",
  error: "Error",
};

const OUTCOME_CLASS: Record<FlightEntry["outcome"], string> = {
  passed: "",
  allowed: "text-safe",
  blocked: "text-threat",
  held: "text-warn",
  error: "text-threat",
};

function matches(e: FlightEntry, text: string, agent: string, llmOnly: boolean): boolean {
  if (agent && e.agent !== agent) return false;
  if (llmOnly && !e.llm) return false;
  if (text) {
    const t = text.toLowerCase();
    const hay = `${e.agent} ${e.host} ${e.path}`.toLowerCase();
    if (!hay.includes(t)) return false;
  }
  return true;
}

export function ActivityView() {
  const desktop = isTauri();
  const [entries, setEntries] = useState<FlightEntry[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [search, setSearch] = useState("");
  const [text, setText] = useState("");
  const [agent, setAgent] = useState("");
  const [llmOnly, setLlmOnly] = useState(false);
  const [agentsSeen, setAgentsSeen] = useState<string[]>([]);
  const [armClear, setArmClear] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  // Debounce the search box.
  useEffect(() => {
    const t = window.setTimeout(() => setText(search.trim()), 200);
    return () => window.clearTimeout(t);
  }, [search]);

  // Remember every agent name we have seen so the filter keeps its options.
  const noteAgents = (list: FlightEntry[]) => {
    setAgentsSeen((prev) => {
      const set = new Set(prev);
      let changed = false;
      for (const e of list) {
        if (e?.agent && !set.has(e.agent)) {
          set.add(e.agent);
          changed = true;
        }
      }
      return changed ? [...set].sort((a, b) => a.localeCompare(b)) : prev;
    });
  };

  // Load (and reload when the filter changes).
  useEffect(() => {
    if (!desktop) return;
    let alive = true;
    const query: FlightQuery = { limit: MAX_SHOWN };
    if (text) query.text = text;
    if (agent) query.agent = agent;
    if (llmOnly) query.llmOnly = true;
    void getFlight(query).then((list) => {
      if (!alive) return;
      const safe = Array.isArray(list) ? list : [];
      setEntries(safe);
      noteAgents(safe);
      setLoaded(true);
    });
    return () => {
      alive = false;
    };
  }, [desktop, text, agent, llmOnly]);

  // Live updates.
  useEffect(() => {
    if (!desktop) return;
    let disposed = false;
    const unlisteners: (() => void)[] = [];
    const keep = (p: Promise<() => void>) => {
      void p.then((un) => {
        if (disposed) un();
        else unlisteners.push(un);
      });
    };
    keep(
      onFlightAdded((e) => {
        if (!e) return;
        noteAgents([e]);
        setEntries((prev) => {
          if (prev.some((x) => x.id === e.id)) return prev.map((x) => (x.id === e.id ? e : x));
          const next = [e, ...prev];
          return next.length > MAX_KEPT ? next.slice(0, MAX_KEPT) : next;
        });
      }),
    );
    keep(
      onFlightUpdated((e) => {
        if (!e) return;
        setEntries((prev) => prev.map((x) => (x.id === e.id ? e : x)));
      }),
    );
    return () => {
      disposed = true;
      unlisteners.forEach((un) => un());
    };
  }, [desktop]);

  // Disarm the clear button after a few seconds.
  useEffect(() => {
    if (!armClear) return;
    const t = window.setTimeout(() => setArmClear(false), 3000);
    return () => window.clearTimeout(t);
  }, [armClear]);

  const filtered = useMemo(
    () => entries.filter((e) => e && matches(e, text, agent, llmOnly)),
    [entries, text, agent, llmOnly],
  );
  const shown = filtered.length > MAX_SHOWN ? filtered.slice(0, MAX_SHOWN) : filtered;

  const onExport = async () => {
    setExporting(true);
    const query: FlightQuery = {};
    if (text) query.text = text;
    if (agent) query.agent = agent;
    if (llmOnly) query.llmOnly = true;
    const res = await exportHar(query);
    setExporting(false);
    if (res.ok && !res.path) return; // cancelled
    if (res.ok) toast.success(`Saved ${res.path}`);
    else toast.error(`Couldn't export. ${res.error ?? ""}`.trim());
  };

  const onClear = async () => {
    if (!armClear) {
      setArmClear(true);
      return;
    }
    setArmClear(false);
    const ok = await clearFlight();
    if (ok) setEntries([]);
    else toast.error("Couldn't clear activity.");
  };

  return (
    <div className="flex h-full flex-col">
      <header className="flex h-12 shrink-0 items-center gap-3 border-b border-border/70 px-5">
        <h1 className="text-[13px] font-semibold">Activity</h1>
        {desktop && (
          <>
            <Input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search agent, host or path"
              className="h-7 max-w-56 text-xs select-text"
            />
            <select
              value={agent}
              onChange={(e) => setAgent(e.target.value)}
              className="h-7 rounded-md border border-input bg-transparent px-2 text-xs text-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              <option value="">All agents</option>
              {agentsSeen.map((a) => (
                <option key={a} value={a}>
                  {a}
                </option>
              ))}
            </select>
            <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
              <Switch checked={llmOnly} onCheckedChange={setLlmOnly} />
              AI calls only
            </label>
            <div className="ml-auto flex items-center gap-1">
              <Button
                variant="outline"
                size="sm"
                className="h-7 text-xs"
                disabled={exporting}
                onClick={() => void onExport()}
              >
                Export HAR
              </Button>
              <Button
                variant="ghost"
                size="sm"
                className={cn("h-7 text-xs", armClear && "text-threat")}
                onClick={() => void onClear()}
              >
                {armClear ? "Click again to clear" : "Clear"}
              </Button>
            </div>
          </>
        )}
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {!desktop ? (
          <Empty text="Desktop app only." />
        ) : shown.length === 0 ? (
          <Empty
            text={
              !loaded
                ? ""
                : entries.length === 0 && !text && !agent && !llmOnly
                  ? "No traffic yet. Requests show up here as soon as an agent runs through Friction."
                  : "No matches."
            }
          />
        ) : (
          <>
            <table className="w-full table-fixed text-xs select-text">
              <thead className="sticky top-0 z-10 bg-background/95 text-left text-[11px] text-muted-foreground backdrop-blur">
                <tr className="border-b border-border/70">
                  <th className="w-20 py-1.5 pl-5 font-medium">Time</th>
                  <th className="w-28 py-1.5 font-medium">Agent</th>
                  <th className="py-1.5 font-medium">Request</th>
                  <th className="w-14 py-1.5 font-medium">Status</th>
                  <th className="w-16 py-1.5 font-medium">Outcome</th>
                  <th className="w-16 py-1.5 text-right font-medium">Size</th>
                  <th className="w-16 py-1.5 text-right font-medium">Time taken</th>
                  <th className="w-20 py-1.5 pr-5 text-right font-medium">Cost</th>
                </tr>
              </thead>
              <tbody>
                {shown.map((e) => (
                  <tr
                    key={e.id}
                    onClick={() => setSelectedId(e.id)}
                    className="cursor-default border-b border-border/50 hover:bg-accent/50"
                  >
                    <td className="py-1.5 pl-5 text-muted-foreground tabular-nums">
                      {formatClock(e.ts)}
                    </td>
                    <td className="truncate py-1.5 pr-2" title={e.agent}>
                      {e.agent}
                    </td>
                    <td
                      className="truncate py-1.5 pr-2 font-mono text-[11px]"
                      title={`${e.method} ${e.host}${e.path}`}
                    >
                      <span className="text-muted-foreground">{e.method}</span> {e.host}
                      <span className="text-muted-foreground">{e.path}</span>
                    </td>
                    <td className={cn("py-1.5 font-mono tabular-nums", statusClass(e.status))}>
                      {e.status ?? "..."}
                    </td>
                    <td
                      className={cn("py-1.5 font-medium", OUTCOME_CLASS[e.outcome])}
                      title={e.reason ?? undefined}
                    >
                      {OUTCOME_LABEL[e.outcome] ?? ""}
                    </td>
                    <td className="py-1.5 text-right text-muted-foreground tabular-nums">
                      {e.status == null && e.respBytes === 0 ? "" : formatBytes(e.respBytes)}
                    </td>
                    <td className="py-1.5 text-right text-muted-foreground tabular-nums">
                      {formatDuration(e.durationMs)}
                    </td>
                    <td className="py-1.5 pr-5 text-right font-mono tabular-nums">{llmCost(e)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {filtered.length > MAX_SHOWN && (
              <p className="px-5 py-3 text-[11px] text-muted-foreground">
                Showing newest {MAX_SHOWN}
              </p>
            )}
          </>
        )}
      </div>

      <FlightSheet id={selectedId} onClose={() => setSelectedId(null)} />
    </div>
  );
}

function Empty({ text }: { text: string }) {
  return (
    <div className="flex h-full items-center justify-center p-8 text-center">
      <p className="max-w-sm text-xs leading-relaxed text-muted-foreground">{text}</p>
    </div>
  );
}

function prettyBody(body: string): { text: string; truncated: boolean } {
  let text = body;
  const trimmed = body.trim();
  if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
    try {
      text = JSON.stringify(JSON.parse(trimmed), null, 2);
    } catch {
      text = body;
    }
  }
  if (text.length > BODY_LIMIT) return { text: text.slice(0, BODY_LIMIT), truncated: true };
  return { text, truncated: false };
}

function Block({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col overflow-hidden rounded-md border bg-background/50">
      <p className="border-b px-3 py-1.5 text-[11px] font-medium text-muted-foreground">{title}</p>
      {children}
    </div>
  );
}

function Pre({ text }: { text: string }) {
  return (
    <pre className="max-h-[40vh] overflow-auto px-3 py-2 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap break-all text-foreground/80 select-text">
      {text}
    </pre>
  );
}

function HeadersBlock({ title, headers }: { title: string; headers: [string, string][] }) {
  const list = Array.isArray(headers) ? headers : [];
  return (
    <Block title={title}>
      {list.length === 0 ? (
        <p className="px-3 py-2 text-xs text-muted-foreground">None</p>
      ) : (
        <Pre text={list.map(([k, v]) => `${k}: ${v}`).join("\n")} />
      )}
    </Block>
  );
}

function BodyBlock({ title, body }: { title: string; body: string | null }) {
  if (!body) {
    return (
      <Block title={title}>
        <p className="px-3 py-2 text-xs text-muted-foreground">Empty</p>
      </Block>
    );
  }
  const { text, truncated } = prettyBody(body);
  return (
    <Block title={title}>
      <Pre text={text} />
      {truncated && (
        <p className="border-t px-3 py-1.5 text-[11px] text-muted-foreground">
          Cut off after {formatBytes(BODY_LIMIT)}.
        </p>
      )}
    </Block>
  );
}

function FlightSheet({ id, onClose }: { id: string | null; onClose: () => void }) {
  const [detail, setDetail] = useState<FlightDetail | null>(null);
  const [missing, setMissing] = useState(false);
  const reqRef = useRef(0);

  useEffect(() => {
    if (!id) {
      setDetail(null);
      setMissing(false);
      return;
    }
    const n = ++reqRef.current;
    setDetail(null);
    setMissing(false);
    void getFlightEntry(id).then((d) => {
      if (n !== reqRef.current) return;
      if (d) setDetail(d);
      else setMissing(true);
    });
  }, [id]);

  const llm = detail?.llm ?? null;
  const outcome = detail ? OUTCOME_LABEL[detail.outcome] || "Passed" : "";

  return (
    <Sheet open={id != null} onOpenChange={(open) => !open && onClose()}>
      <SheetContent side="right" className="w-full overflow-y-auto p-5 sm:max-w-2xl">
        {!detail ? (
          <>
            <SheetTitle className="text-[13px]">Request</SheetTitle>
            <SheetDescription className="text-xs">
              {missing ? "This request is no longer in the log." : "Loading..."}
            </SheetDescription>
          </>
        ) : (
          <div className="space-y-4">
            <div className="pr-6">
              <SheetTitle className="font-mono text-[13px] break-all">
                {detail.method} {detail.url}
              </SheetTitle>
              <SheetDescription className="mt-1 text-xs tabular-nums">
                {formatClock(detail.ts)} · {detail.agent} · Status {detail.status ?? "pending"} ·{" "}
                <span className={OUTCOME_CLASS[detail.outcome]}>{outcome}</span>
                {detail.durationMs != null && ` · ${formatDuration(detail.durationMs)}`} · sent{" "}
                {formatBytes(detail.reqBytes)}, received {formatBytes(detail.respBytes)}
              </SheetDescription>
              {detail.reason && (
                <p className="mt-1 text-xs text-muted-foreground">{detail.reason}</p>
              )}
            </div>

            {llm && (
              <p className="text-xs tabular-nums">
                <span className="text-muted-foreground">AI call:</span> {llm.provider}
                {llm.model ? ` · ${llm.model}` : ""} · {llm.inputTokens.toLocaleString()} in,{" "}
                {llm.outputTokens.toLocaleString()} out
                {llm.cachedInputTokens > 0 &&
                  `, ${llm.cachedInputTokens.toLocaleString()} cached`}{" "}
                ·{" "}
                {llm.costUsd == null
                  ? "unknown price"
                  : `${llm.estimated ? "≈" : ""}${formatUsd(llm.costUsd)}`}
              </p>
            )}

            {Array.isArray(detail.threats) && detail.threats.length > 0 && (
              <div>
                <p className="mb-1 text-xs font-medium text-threat">Issues found</p>
                <ul className="list-disc space-y-0.5 pl-4 text-xs">
                  {detail.threats.map((t, i) => (
                    <li key={i}>{t}</li>
                  ))}
                </ul>
              </div>
            )}

            <HeadersBlock title="Request headers" headers={detail.requestHeaders} />
            {detail.bodiesSaved ? (
              <BodyBlock title="Request body" body={detail.requestBody} />
            ) : null}
            <HeadersBlock title="Response headers" headers={detail.responseHeaders} />
            {detail.bodiesSaved ? (
              <BodyBlock title="Response body" body={detail.responseBody} />
            ) : (
              <p className="text-xs text-muted-foreground">
                Bodies aren't saved. Turn on Rules › Save request bodies to keep them.
              </p>
            )}
          </div>
        )}
      </SheetContent>
    </Sheet>
  );
}
