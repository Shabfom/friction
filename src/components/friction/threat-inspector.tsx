import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { PayloadBlock } from "./payload-block";
import { SeverityDot } from "./severity-badge";
import type { Intercept, Threat } from "@/lib/friction-data";

function useCountdown(expiresAt: number | undefined) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!expiresAt) return;
    setNow(Date.now());
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, [expiresAt]);
  if (!expiresAt) return null;
  const left = Math.max(0, Math.round((expiresAt - now) / 1000));
  return `${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")}`;
}

function rank(t: Threat) {
  return t.severity === "critical" ? 0 : t.severity === "warning" ? 1 : 2;
}

/** Review sheet for a held request. ⌘↩ allows, ⌘⌫ blocks, Esc closes. */
export function ThreatInspector({
  intercept,
  onOpenChange,
  onApprove,
  onReject,
}: {
  intercept: Intercept | null;
  onOpenChange: (open: boolean) => void;
  onApprove: (id: string) => void;
  onReject: (id: string) => void;
}) {
  const countdown = useCountdown(intercept?.expiresAt);

  useEffect(() => {
    if (!intercept) return;
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey) return;
      if (e.key === "Enter") {
        e.preventDefault();
        onApprove(intercept.id);
      } else if (e.key === "Backspace") {
        e.preventDefault();
        onReject(intercept.id);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [intercept, onApprove, onReject]);

  if (!intercept) return <Dialog open={false} onOpenChange={onOpenChange} />;

  const findings = (intercept.threats ?? []).slice().sort((a, b) => rank(a) - rank(b));

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[88vh] gap-0 overflow-y-auto p-0 sm:max-w-3xl">
        <DialogHeader className="space-y-0 border-b px-5 py-4 text-left">
          <div className="flex items-start justify-between gap-6 pr-6">
            <div className="min-w-0">
              <DialogDescription className="text-xs">
                {intercept.agent} → {intercept.destination}
              </DialogDescription>
              <DialogTitle className="mt-0.5 truncate text-base font-semibold select-text">
                {intercept.summary}
              </DialogTitle>
            </div>
            {intercept.amount && (
              <p className="shrink-0 font-mono text-lg tabular-nums">{intercept.amount}</p>
            )}
          </div>
        </DialogHeader>

        <ul className="space-y-2.5 border-b px-5 py-4">
          {findings.map((t) => (
            <li key={t.code} className="flex gap-2.5">
              <SeverityDot severity={t.severity} className="mt-1.5" />
              <div className="min-w-0 text-xs leading-relaxed select-text">
                <span className="font-medium text-foreground">{t.label}</span>
                <p className="text-muted-foreground">{t.detail}</p>
              </div>
            </li>
          ))}
        </ul>

        <div className="px-5 py-4">
          <PayloadBlock
            title="What the agent is sending"
            lines={intercept.executionPayload}
            highlight
          />
        </div>

        <div className="flex items-center justify-between gap-4 border-t px-5 py-3">
          <p className="text-[11px] text-muted-foreground tabular-nums">
            {countdown ? `Blocked automatically in ${countdown}` : ""}
          </p>
          <div className="flex gap-2">
            <Button
              variant="outline"
              className="text-threat hover:text-threat"
              onClick={() => onReject(intercept.id)}
            >
              Block
              <kbd className="ml-1 font-sans text-[11px] text-muted-foreground">⌘⌫</kbd>
            </Button>
            <Button onClick={() => onApprove(intercept.id)}>
              Allow
              <kbd className="ml-1 font-sans text-[11px] opacity-60">⌘↩</kbd>
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
