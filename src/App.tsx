import { useEffect, useState } from "react";
import { AppSidebar, type ViewKey } from "@/components/friction/app-sidebar";
import { InterceptsView } from "@/components/friction/intercepts-view";
import { AuditLogView } from "@/components/friction/audit-log-view";
import { RulesView } from "@/components/friction/rules-view";
import { ActivityView } from "@/components/friction/activity-view";
import { SpendView } from "@/components/friction/spend-view";
import { ThreatInspector } from "@/components/friction/threat-inspector";
import { useFriction } from "@/lib/friction-store";
import { useTheme } from "@/hooks/use-theme";
import { requestNotificationPermission } from "@/lib/notifications";
import { Toaster } from "@/components/ui/sonner";
import type { Intercept } from "@/lib/friction-data";
import { getCaInfo, getStats, onNavigate, type Stats } from "@/lib/tauri-ipc";

const VIEWS: ViewKey[] = ["intercepts", "activity", "spend", "audit", "rules"];

export default function FrictionApp() {
  useTheme();
  const f = useFriction();
  const [view, setView] = useState<ViewKey>("intercepts");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [stats, setStats] = useState<Stats | null>(null);
  const [caTrusted, setCaTrusted] = useState(true);
  const intercepts = Array.isArray(f.intercepts) ? f.intercepts : [];
  const proxyDown = f.proxyStatus !== null && f.proxyStatus.kind !== "ok";

  useEffect(() => void requestNotificationPermission(), []);

  // Live counters for the sidebar, and certificate state for the setup checklist.
  useEffect(() => {
    if (!f.desktop) return;
    const tick = () => {
      void getStats().then((s) => s && setStats(s));
      void getCaInfo().then((ca) => setCaTrusted(!ca || !ca.available || ca.trusted));
    };
    tick();
    const t = window.setInterval(tick, 2000);
    return () => window.clearInterval(t);
  }, [f.desktop]);

  // App menu "Settings…" (⌘,) and the menu bar icon switch views.
  useEffect(() => {
    const unlisten = onNavigate((next) => {
      if ((VIEWS as string[]).includes(next)) {
        setSelectedId(null);
        setView(next as ViewKey);
      }
    });
    return () => void unlisten.then((fn) => fn());
  }, []);

  // ⌘1 to ⌘5 switch views.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!e.metaKey && !e.ctrlKey) return;
      const next = VIEWS[Number(e.key) - 1];
      if (next) {
        e.preventDefault();
        setView(next);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  const selected: Intercept | null = intercepts.find((i) => i.id === selectedId) ?? null;
  const close = (fn: (id: string) => void) => (id: string) => {
    fn(id);
    setSelectedId(null);
  };

  return (
    <div className="flex h-screen w-screen flex-col overflow-hidden bg-background/80 text-[13px]">
      {proxyDown && (
        <div className="shrink-0 border-b border-threat/30 bg-threat-muted px-4 py-2 text-xs text-threat select-none">
          {f.proxyStatus?.kind === "port_in_use"
            ? "Ports 8080–8099 are all in use by other apps. Free one, then reopen Friction."
            : `Friction's proxy couldn't start: ${f.proxyStatus?.message ?? "unknown error"}`}
        </div>
      )}

      <div className="flex min-h-0 flex-1">
        <AppSidebar
          view={view}
          onChange={setView}
          pendingCount={intercepts.length}
          desktop={f.desktop}
          proxyStatus={f.proxyStatus}
          stats={stats}
        />

        <main className="min-w-0 flex-1 overflow-hidden bg-background/50">
          {view === "intercepts" && (
            <InterceptsView
              intercepts={intercepts}
              onSelect={(i) => setSelectedId(i.id)}
              desktop={f.desktop}
              showSetup={!caTrusted || !f.seenTraffic}
              seenTraffic={f.seenTraffic}
            />
          )}
          {view === "activity" && <ActivityView />}
          {view === "spend" && <SpendView />}
          {view === "audit" && <AuditLogView audit={Array.isArray(f.audit) ? f.audit : []} />}
          {view === "rules" && (
            <RulesView
              desktop={f.desktop}
              rules={f.rules}
              fileOverrides={f.fileOverrides}
              onToggle={f.toggleRule}
              onClearHistory={f.clearHistory}
              onRestoreRules={f.restoreDefaultRules}
            />
          )}
        </main>
      </div>

      <Toaster position="bottom-right" />

      <ThreatInspector
        intercept={selected}
        onOpenChange={(open) => !open && setSelectedId(null)}
        onApprove={close(f.approve)}
        onReject={close(f.reject)}
      />
    </div>
  );
}
