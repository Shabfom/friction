import { cn } from "@/lib/utils";
import { normalizeLine, type PayloadLine } from "@/lib/friction-data";

export function PayloadBlock({
  title,
  lines,
  highlight = false,
}: {
  title: string;
  lines: (PayloadLine | string)[];
  highlight?: boolean;
}) {
  return (
    <div className="flex min-w-0 flex-col overflow-hidden rounded-md border bg-background/50">
      <p className="border-b px-3 py-1.5 text-[11px] font-medium text-muted-foreground">{title}</p>
      <pre className="max-h-[34vh] overflow-auto py-2 font-mono text-[11.5px] leading-relaxed select-text">
        {lines.map(normalizeLine).map((line, i) => (
          <div
            key={i}
            className={cn(
              "border-l-2 border-transparent px-3 whitespace-pre-wrap",
              line.flagged && highlight
                ? "border-l-threat bg-threat-muted text-threat"
                : "text-foreground/80",
            )}
          >
            {line.text}
          </div>
        ))}
      </pre>
    </div>
  );
}
