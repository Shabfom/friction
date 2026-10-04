export function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section>
      <h2 className="mb-2 px-1 text-xs font-medium text-muted-foreground">{title}</h2>
      <div className="divide-y rounded-lg border bg-card">{children}</div>
    </section>
  );
}

export function Row({
  title,
  detail,
  children,
}: {
  title: string;
  detail?: React.ReactNode;
  children?: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-6 px-4 py-3">
      <div className="min-w-0">
        <p className="text-[13px]">{title}</p>
        {detail && <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">{detail}</p>}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}
