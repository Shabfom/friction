import { useState } from "react";
import { Button } from "@/components/ui/button";
import { sendDemoRequest } from "@/lib/tauri-ipc";
import { copyText, proxyEnv, trustCertificate, useCa, useCli, useProxyAddr } from "@/lib/setup";
import { cn } from "@/lib/utils";
import { Check, Copy } from "lucide-react";

function Step({
  n,
  done,
  title,
  children,
}: {
  n: number;
  done: boolean;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <li className="flex gap-3">
      <span
        className={cn(
          "mt-px flex size-5 shrink-0 items-center justify-center rounded-full border text-[11px] tabular-nums",
          done ? "border-safe bg-safe text-safe-foreground" : "border-border text-muted-foreground",
        )}
      >
        {done ? <Check className="size-3" strokeWidth={3} /> : n}
      </span>
      <div className="min-w-0 flex-1">
        <p className={cn("text-[13px] font-medium", done && "text-muted-foreground")}>{title}</p>
        <div className="mt-1 text-xs leading-relaxed text-muted-foreground">{children}</div>
      </div>
    </li>
  );
}

export function SetupChecklist({ seenTraffic }: { seenTraffic: boolean }) {
  const { ca, refresh } = useCa();
  const [busy, setBusy] = useState(false);
  const [demoSent, setDemoSent] = useState(false);
  const [showVars, setShowVars] = useState(false);
  const cli = useCli();
  const addr = useProxyAddr();
  const env = proxyEnv(ca?.path ?? "~/.friction/friction-root-ca.pem", addr);
  const runLine = "friction run -- python agent.py";

  return (
    <div className="w-full max-w-lg rounded-lg border bg-card p-5 text-left">
      <p className="text-sm font-semibold">Set up Friction</p>
      <p className="mt-1 text-xs text-muted-foreground">
        Friction is a local proxy. Agents send their traffic through it, and it holds anything risky
        until you decide.
      </p>

      <ol className="mt-5 space-y-5">
        <Step n={1} done={!!ca?.trusted} title="Trust the certificate">
          {ca?.trusted ? (
            "Done. Friction can read HTTPS requests."
          ) : (
            <>
              Lets Friction read HTTPS requests. It was created on this Mac and never leaves it.
              macOS will ask for your password.
              <div className="mt-2">
                <Button
                  size="sm"
                  disabled={busy || !ca?.available}
                  onClick={async () => {
                    setBusy(true);
                    await trustCertificate(refresh);
                    setBusy(false);
                  }}
                >
                  {busy ? "Waiting for macOS…" : "Trust certificate"}
                </Button>
              </div>
            </>
          )}
        </Step>

        <Step n={2} done={seenTraffic} title="Run your agent through Friction">
          {seenTraffic ? (
            "Done. Friction has seen traffic from an agent."
          ) : (
            <>
              {cli.path ? (
                <>Start your agent with the friction command. Any language works.</>
              ) : (
                <>
                  Install the friction command, then start your agent with it.
                  <div className="mt-2">
                    <Button size="sm" variant="outline" onClick={() => void cli.install()}>
                      Install command
                    </Button>
                  </div>
                </>
              )}
              <CodeLine text={runLine} />
              <button
                type="button"
                onClick={() => setShowVars((v) => !v)}
                className="mt-2 underline-offset-4 hover:text-foreground hover:underline"
              >
                {showVars ? "Hide shell variables" : "Or set the variables yourself"}
              </button>
              {showVars && <CodeLine text={env} />}
            </>
          )}
        </Step>

        <Step n={3} done={demoSent} title="See it in action">
          Send a fake request from a demo agent. It shows up here for you to allow or block. Nothing
          leaves your Mac.
          <div className="mt-2 flex gap-2">
            <Button
              size="sm"
              variant="outline"
              onClick={() => {
                setDemoSent(true);
                void sendDemoRequest("purchase");
              }}
            >
              A $279 payment
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={() => {
                setDemoSent(true);
                void sendDemoRequest("leak");
              }}
            >
              Leaked API key
            </Button>
          </div>
        </Step>
      </ol>
    </div>
  );
}

function CodeLine({ text }: { text: string }) {
  return (
    <div className="relative mt-2">
      <pre className="overflow-x-auto rounded-md border bg-background/60 p-2.5 pr-9 font-mono text-[11px] leading-relaxed text-foreground select-text">
        {text}
      </pre>
      <button
        type="button"
        aria-label="Copy"
        onClick={() => void copyText(text, "to clipboard")}
        className="absolute top-1.5 right-1.5 rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
      >
        <Copy className="size-3.5" />
      </button>
    </div>
  );
}
