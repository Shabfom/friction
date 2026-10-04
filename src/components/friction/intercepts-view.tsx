import { SeverityDot } from "./severity-badge";
import { SetupChecklist } from "./setup-checklist";
import { formatClock, type Intercept } from "@/lib/friction-data";
import { sendDemoRequest } from "@/lib/tauri-ipc";
import { ShieldCheck } from "lucide-react";

export function InterceptsView({
  intercepts,
  onSelect,
  desktop,
  showSetup,
  seenTraffic,
}: {
  intercepts: Intercept[];
  onSelect: (intercept: Intercept) => void;
  desktop: boolean;
  showSetup: boolean;
  seenTraffic: boolean;
}) {
  const list = Array.isArray(intercepts) ? intercepts : [];

  return (
    <div className="flex h-full flex-col">
      <header className="flex h-12 shrink-0 items-center justify-between border-b border-border/70 px-5">
        <h1 className="text-[13px] font-semibold">Requests</h1>
        {list.length > 0 && (
          <span className="text-xs text-muted-foreground">{list.length} waiting for you</span>
        )}
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {list.length === 0 ? (
          <div className="flex min-h-full flex-col items-center justify-center gap-2 p-8 text-center">
            {desktop && showSetup ? (
              <SetupChecklist seenTraffic={seenTraffic} />
            ) : (
              <>
                <ShieldCheck className="size-7 text-muted-foreground/60" strokeWidth={1.5} />
                <p className="mt-1 text-sm font-medium">Nothing waiting</p>
                <p className="max-w-xs text-xs leading-relaxed text-muted-foreground">
                  Requests that need your decision show up here. Everything else goes straight
                  through.
                </p>
                {desktop && (
                  <p className="mt-2 text-xs text-muted-foreground">
                    Try it:{" "}
                    <button
                      type="button"
                      onClick={() => void sendDemoRequest("purchase")}
                      className="underline-offset-4 hover:text-foreground hover:underline"
                    >
                      a $279 payment
                    </button>{" "}
                    or{" "}
                    <button
                      type="button"
                      onClick={() => void sendDemoRequest("leak")}
                      className="underline-offset-4 hover:text-foreground hover:underline"
                    >
                      a leaked API key
                    </button>
                  </p>
                )}
              </>
            )}
          </div>
        ) : (
          <table className="w-full text-left text-xs">
            <thead className="sticky top-0 z-10 bg-background/95 text-[11px] text-muted-foreground backdrop-blur">
              <tr className="border-b">
                <Th>Agent</Th>
                <Th>Going to</Th>
                <Th>Request</Th>
                <Th className="text-right">Amount</Th>
                <Th>Why it's held</Th>
                <Th className="text-right">Time</Th>
              </tr>
            </thead>
            <tbody>
              {list.map((i) => {
                const threats = (Array.isArray(i?.threats) ? i.threats : []).filter(
                  (t) => t.severity !== "info",
                );
                const worst = threats.find((t) => t.severity === "critical") ?? threats[0];
                return (
                  <tr
                    key={i.id}
                    role="button"
                    tabIndex={0}
                    aria-label={`Review ${i.agent} → ${i.destination}: ${i.summary}`}
                    onClick={() => onSelect(i)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        onSelect(i);
                      }
                    }}
                    className="cursor-default border-b border-border/50 transition-colors hover:bg-accent/60 focus-visible:bg-accent/60 focus-visible:outline-none"
                  >
                    <Td className="font-medium text-foreground">{i.agent}</Td>
                    <Td>{i.destination}</Td>
                    <Td className="max-w-[28ch] truncate text-foreground">{i.summary}</Td>
                    <Td className="text-right font-mono text-foreground tabular-nums">
                      {i.amount ?? <span className="text-muted-foreground/60">–</span>}
                    </Td>
                    <Td>
                      <span className="flex items-center gap-2">
                        {worst && <SeverityDot severity={worst.severity} />}
                        <span className="truncate text-foreground">{worst?.label}</span>
                        {threats.length > 1 && (
                          <span className="shrink-0 text-muted-foreground">
                            +{threats.length - 1}
                          </span>
                        )}
                      </span>
                    </Td>
                    <Td className="text-right tabular-nums">{formatClock(i.timestamp)}</Td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}

function Th({ children, className = "" }: { children?: React.ReactNode; className?: string }) {
  return <th className={"px-4 py-2 font-medium " + className}>{children}</th>;
}

function Td({ children, className = "" }: { children?: React.ReactNode; className?: string }) {
  return <td className={"px-4 py-2.5 align-top text-muted-foreground " + className}>{children}</td>;
}
