import { cn } from "@/lib/utils";
import {
  Activity,
  CircleDollarSign,
  History,
  Inbox,
  SlidersHorizontal,
  ShieldHalf,
} from "lucide-react";
import type { ProxyStatus, Stats } from "@/lib/tauri-ipc";

export type ViewKey = "intercepts" | "activity" | "spend" | "audit" | "rules";

const NAV: { key: ViewKey; label: string; icon: typeof Inbox; shortcut: string }[] = [
  { key: "intercepts", label: "Requests", icon: Inbox, shortcut: "⌘1" },
  { key: "activity", label: "Activity", icon: Activity, shortcut: "⌘2" },
  { key: "spend", label: "AI spend", icon: CircleDollarSign, shortcut: "⌘3" },
  { key: "audit", label: "Decisions", icon: History, shortcut: "⌘4" },
  { key: "rules", label: "Rules", icon: SlidersHorizontal, shortcut: "⌘5" },
];

const fmt = new Intl.NumberFormat("en-US");

export function AppSidebar({
  view,
  onChange,
  pendingCount,
  desktop,
  proxyStatus,
  stats,
}: {
  view: ViewKey;
  onChange: (view: ViewKey) => void;
  pendingCount: number;
  desktop: boolean;
  proxyStatus: ProxyStatus | null;
  stats: Stats | null;
}) {
  const ok = proxyStatus?.kind === "ok";
  return (
    <aside className="flex w-[196px] shrink-0 flex-col border-r border-border/70 bg-sidebar select-none">
      <div className="flex items-center gap-2 px-4 pt-4 pb-3">
        <ShieldHalf className="size-4 text-threat" />
        <span className="text-[13px] font-semibold tracking-tight">Friction</span>
      </div>

      <nav className="flex flex-col gap-px px-2">
        {NAV.map((item) => {
          const active = view === item.key;
          return (
            <button
              key={item.key}
              type="button"
              title={item.shortcut}
              onClick={() => onChange(item.key)}
              className={cn(
                "flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors",
                active
                  ? "bg-sidebar-accent text-sidebar-accent-foreground"
                  : "text-muted-foreground hover:bg-sidebar-accent/50 hover:text-sidebar-foreground",
              )}
            >
              <item.icon className="size-4" />
              <span className="flex-1 truncate">{item.label}</span>
              {item.key === "intercepts" && pendingCount > 0 && (
                <span className="min-w-5 rounded-full bg-threat px-1.5 text-center text-[11px] font-medium text-threat-foreground tabular-nums">
                  {pendingCount}
                </span>
              )}
            </button>
          );
        })}
      </nav>

      <div className="mt-auto space-y-1 px-4 py-3 text-[11px] text-muted-foreground">
        {desktop ? (
          <>
            <p className="flex items-center gap-1.5">
              <span
                className={cn(
                  "size-1.5 rounded-full",
                  ok ? "bg-safe" : proxyStatus ? "bg-threat" : "bg-muted-foreground/50",
                )}
              />
              {ok ? `Listening on :${proxyStatus?.addr.split(":").pop()}` : proxyStatus ? "Proxy stopped" : "Starting…"}
            </p>
            {stats && stats.inspected > 0 && (
              <p className="tabular-nums">
                {fmt.format(stats.inspected)} checked · {fmt.format(stats.blocked)} blocked
              </p>
            )}
          </>
        ) : (
          <p>Browser preview with sample data</p>
        )}
      </div>
    </aside>
  );
}
