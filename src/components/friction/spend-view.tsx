import { useEffect, useRef, useState } from "react";
import { Input } from "@/components/ui/input";
import { Row, Section } from "./settings-ui";
import { isTauri } from "@/lib/tauri-ipc";
import { getSpend, onSpendChanged, setAiCap, type SpendSummary } from "@/lib/activity-ipc";
import { cn } from "@/lib/utils";

function money(v: number): string {
  if (!Number.isFinite(v)) return "$0.00";
  if (v > 0 && v < 0.01) return `$${v.toFixed(4)}`;
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(v);
}

function dayLabel(date: string): string {
  // date is YYYY-MM-DD; parse as local so the day does not shift.
  const [y, m, d] = date.split("-").map(Number);
  if (!y || !m || !d) return date;
  return new Date(y, m - 1, d).toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

export function SpendView() {
  const desktop = isTauri();
  const [spend, setSpend] = useState<SpendSummary | null>(null);
  const [draft, setDraft] = useState("");
  const editing = useRef(false);

  const apply = (s: SpendSummary | null) => {
    if (!s) return;
    setSpend(s);
    if (!editing.current) setDraft(s.cap == null ? "" : String(s.cap));
  };

  useEffect(() => {
    if (!desktop) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    const refresh = () =>
      void getSpend().then((s) => {
        if (!disposed) apply(s);
      });
    refresh();
    const timer = window.setInterval(refresh, 5000);
    void onSpendChanged((s) => {
      if (!disposed) apply(s);
    }).then((un) => {
      if (disposed) un();
      else unlisten = un;
    });
    return () => {
      disposed = true;
      window.clearInterval(timer);
      unlisten?.();
    };
  }, [desktop]);

  const commit = () => {
    editing.current = false;
    if (!spend || spend.capFromRulesFile) return;
    const raw = draft.trim();
    const next = raw === "" ? null : Number(raw);
    if (next != null && (!Number.isFinite(next) || next < 0)) {
      setDraft(spend.cap == null ? "" : String(spend.cap));
      return;
    }
    if (next === spend.cap) return;
    void setAiCap(next).then(apply);
  };

  const header = (
    <header className="flex h-12 shrink-0 items-center border-b border-border/70 px-5">
      <h1 className="text-[13px] font-semibold">Spend</h1>
    </header>
  );

  if (!desktop || !spend) {
    return (
      <div className="flex h-full flex-col">
        {header}
        <div className="flex min-h-0 flex-1 items-center justify-center p-8">
          <p className="text-xs text-muted-foreground">{desktop ? "" : "Desktop app only."}</p>
        </div>
      </div>
    );
  }

  const days = Array.isArray(spend.days) ? spend.days : [];
  const byModel = Array.isArray(spend.byModel) ? spend.byModel : [];
  const byAgent = Array.isArray(spend.byAgent) ? spend.byAgent : [];
  const unpriced = Array.isArray(spend.unpricedModels) ? spend.unpricedModels : [];
  const empty = spend.today === 0 && days.every((d) => !d.costUsd) && byModel.length === 0;
  const cap = spend.cap;
  const pct = cap && cap > 0 ? Math.min(100, (spend.today / cap) * 100) : 0;
  const maxDay = Math.max(0, ...days.map((d) => d.costUsd || 0));

  return (
    <div className="flex h-full flex-col">
      {header}
      <div className="min-h-0 flex-1 overflow-y-auto px-5 py-5">
        <div className="mx-auto max-w-3xl space-y-6">
          <div>
            <p className="font-mono text-3xl font-semibold tabular-nums">
              {spend.todayEstimated ? "≈" : ""}
              {money(spend.today)}
            </p>
            <p className="mt-1 text-xs text-muted-foreground">
              {cap != null ? `of ${money(cap)} daily limit` : "No daily limit"}
            </p>
            {cap != null && (
              <div className="mt-3 h-1.5 w-full overflow-hidden rounded-full bg-muted">
                <div
                  className={cn(
                    "h-full rounded-full",
                    spend.capReached ? "bg-threat" : pct > 80 ? "bg-warn" : "bg-foreground/60",
                  )}
                  style={{ width: `${spend.capReached ? 100 : pct}%` }}
                />
              </div>
            )}
            {spend.capReached && (
              <p className="mt-3 text-xs text-threat">
                Daily limit reached. AI calls are blocked until midnight or until you raise the
                limit.
              </p>
            )}
          </div>

          <Section title="Limit">
            <Row
              title="Daily limit"
              detail={
                spend.capFromRulesFile
                  ? "Set in rules.toml"
                  : "AI calls are blocked once today's spend reaches this. Leave empty for no limit."
              }
            >
              <div className="flex items-center gap-1.5">
                <span className="text-xs text-muted-foreground">$</span>
                <Input
                  type="number"
                  min={0}
                  step="1"
                  value={draft}
                  placeholder="None"
                  disabled={spend.capFromRulesFile}
                  onFocus={() => {
                    editing.current = true;
                  }}
                  onChange={(e) => setDraft(e.target.value)}
                  onBlur={commit}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") e.currentTarget.blur();
                  }}
                  className="h-7 w-24 text-right font-mono text-xs select-text"
                />
              </div>
            </Row>
          </Section>

          {empty ? (
            <p className="py-6 text-center text-xs leading-relaxed text-muted-foreground">
              No AI API calls yet today. Friction meters OpenAI, Anthropic, Google, OpenRouter and
              other AI APIs automatically.
            </p>
          ) : (
            <>
              {days.length > 0 && (
                <Section title="Last 14 days">
                  <div className="px-4 py-3">
                    <div className="flex h-24 items-end gap-1">
                      {days.map((d) => {
                        const h = maxDay > 0 ? ((d.costUsd || 0) / maxDay) * 100 : 0;
                        return (
                          <div
                            key={d.date}
                            title={`${dayLabel(d.date)}: ${money(d.costUsd || 0)}`}
                            className="flex h-full flex-1 items-end"
                          >
                            <div
                              className="w-full rounded-sm bg-foreground/50 hover:bg-foreground/70"
                              style={{ height: `${Math.max(h, d.costUsd > 0 ? 2 : 0)}%` }}
                            />
                          </div>
                        );
                      })}
                    </div>
                    <div className="mt-1.5 flex justify-between text-[11px] text-muted-foreground tabular-nums">
                      <span>{dayLabel(days[0]?.date ?? "")}</span>
                      <span>{dayLabel(days[days.length - 1]?.date ?? "")}</span>
                    </div>
                  </div>
                </Section>
              )}

              {byModel.length > 0 && (
                <Section title="By model">
                  <table className="w-full text-xs select-text">
                    <thead className="text-left text-[11px] text-muted-foreground">
                      <tr>
                        <th className="py-2 pl-4 font-medium">Provider</th>
                        <th className="py-2 font-medium">Model</th>
                        <th className="py-2 text-right font-medium">Calls</th>
                        <th className="py-2 text-right font-medium">Tokens in</th>
                        <th className="py-2 text-right font-medium">Tokens out</th>
                        <th className="py-2 pr-4 text-right font-medium">Cost</th>
                      </tr>
                    </thead>
                    <tbody>
                      {byModel.map((r) => (
                        <tr key={`${r.provider}/${r.model}`} className="border-t">
                          <td className="py-2 pl-4">{r.provider}</td>
                          <td
                            className="max-w-48 truncate py-2 font-mono text-[11px]"
                            title={r.model}
                          >
                            {r.model}
                          </td>
                          <td className="py-2 text-right tabular-nums">
                            {r.calls.toLocaleString()}
                          </td>
                          <td className="py-2 text-right tabular-nums">
                            {r.inputTokens.toLocaleString()}
                          </td>
                          <td className="py-2 text-right tabular-nums">
                            {r.outputTokens.toLocaleString()}
                          </td>
                          <td className="py-2 pr-4 text-right font-mono tabular-nums">
                            {r.costUsd == null ? (
                              <span className="font-sans text-muted-foreground">unknown price</span>
                            ) : (
                              money(r.costUsd)
                            )}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </Section>
              )}

              {byAgent.length > 0 && (
                <Section title="By agent">
                  <table className="w-full text-xs select-text">
                    <thead className="text-left text-[11px] text-muted-foreground">
                      <tr>
                        <th className="py-2 pl-4 font-medium">Agent</th>
                        <th className="py-2 text-right font-medium">Calls</th>
                        <th className="py-2 pr-4 text-right font-medium">Cost</th>
                      </tr>
                    </thead>
                    <tbody>
                      {byAgent.map((r) => (
                        <tr key={r.agent} className="border-t">
                          <td className="py-2 pl-4">{r.agent}</td>
                          <td className="py-2 text-right tabular-nums">
                            {r.calls.toLocaleString()}
                          </td>
                          <td className="py-2 pr-4 text-right font-mono tabular-nums">
                            {money(r.costUsd)}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </Section>
              )}
            </>
          )}

          <div className="space-y-1 px-1 text-[11px] text-muted-foreground">
            <p>Costs use list prices as of {spend.pricesAsOf}. Override prices in rules.toml.</p>
            {unpriced.length > 0 && <p>No price for: {unpriced.join(", ")}</p>}
          </div>
        </div>
      </div>
    </div>
  );
}
