import { cn } from "@/lib/utils";
import type { ThreatSeverity } from "@/lib/friction-data";

const DOT: Record<ThreatSeverity, string> = {
  critical: "bg-threat",
  warning: "bg-warn",
  info: "bg-muted-foreground/60",
};

export function SeverityDot({
  severity,
  className,
}: {
  severity: ThreatSeverity;
  className?: string;
}) {
  return (
    <span className={cn("inline-block size-1.5 shrink-0 rounded-full", DOT[severity], className)} />
  );
}
