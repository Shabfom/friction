import { useState } from "react";
import { formatCurrency, type AuditEntry } from "@/lib/friction-data";
import { cn } from "@/lib/utils";

type Filter = "all" | "rejected" | "approved";

const when = (ts: number) =>
  new Date(ts || Date.now()).toLocaleString("en-US", {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });

export function AuditLogView({ audit }: { audit: AuditEntry[] }) {
  const [filter, setFilter] = useState<Filter>("all");
  const all = Array.isArray(audit) ? audit : [];
  const list = filter === "all" ? all : all.filter((a) => a?.outcome === filter);
  const saved = all
    .filter((a) => a?.outcome === "rejected")
    .reduce((sum, a) => sum + Math.max(0, a?.delta || 0), 0);

  return (
    <div className="flex h-full flex-col">
      <header className="flex h-12 shrink-0 items-center justify-between gap-4 border-b border-border/70 px-5">
        <div className="flex items-center gap-4">
          <h1 className="text-[13px] font-semibold">Decisions</h1>
          <div className="flex rounded-md border p-0.5 text-[11px]">
            {(
              [
                ["all", "All"],
                ["rejected", "Blocked"],
                ["approved", "Allowed"],
              ] as const
            ).map(([key, label]) => (
              <button
                key={key}
                type="button"
                onClick={() => setFilter(key)}
                className={cn(
                  "rounded px-2 py-0.5",
                  filter === key
                    ? "bg-accent text-foreground"
                    : "text-muted-foreground hover:text-foreground",
                )}
              >
                {label}
              </button>
            ))}
          </div>
        </div>
        {saved > 0 && (
          <span className="text-xs text-muted-foreground">
            Payments stopped:{" "}
            <span className="font-mono text-foreground tabular-nums">{formatCurrency(saved)}</span>
          </span>
        )}
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {list.length === 0 ? (
          <div className="flex h-full items-center justify-center">
            <p className="text-xs text-muted-foreground">
              {all.length === 0
                ? "Nothing yet. Decisions you and Friction make show up here."
                : "No matches."}
            </p>
          </div>
        ) : (
          <ul className="select-text">
            {list.map((entry) => {
              if (!entry) return null;
              const blocked = entry.outcome === "rejected";
              const threats = typeof entry.threatsCaught === "number" ? entry.threatsCaught : 0;
              return (
                <li
                  key={entry.id}
                  className="flex items-baseline gap-3 border-b border-border/50 px-5 py-2.5"
                >
                  <span
                    className={cn(
                      "w-16 shrink-0 text-[11px] font-medium",
                      blocked ? "text-threat" : "text-safe",
                    )}
                  >
                    {blocked ? "Blocked" : "Allowed"}
                  </span>
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-xs text-foreground">{entry.item}</p>
                    <p className="mt-0.5 truncate text-[11px] text-muted-foreground">
                      {entry.agent} → {entry.merchant}
                      {entry.reason
                        ? ` · ${entry.reason}`
                        : threats > 0
                          ? ` · ${threats} issue${threats === 1 ? "" : "s"}, ${blocked ? "you blocked it" : "you allowed it"}`
                          : ""}
                    </p>
                  </div>
                  <span className="shrink-0 font-mono text-xs tabular-nums">
                    {blocked ? (
                      entry.delta > 0 ? (
                        <span className="text-muted-foreground line-through">
                          {formatCurrency(entry.delta)}
                        </span>
                      ) : null
                    ) : entry.amount > 0 ? (
                      formatCurrency(entry.amount)
                    ) : null}
                  </span>
                  <span className="w-28 shrink-0 text-right text-[11px] text-muted-foreground tabular-nums">
                    {when(entry.timestamp)}
                  </span>
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </div>
  );
}
