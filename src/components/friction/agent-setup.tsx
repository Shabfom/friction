import { useState } from "react";
import { Button } from "@/components/ui/button";
import { isTauri, revealCa } from "@/lib/tauri-ipc";
import { copyText, proxyEnv, trustCertificate, useCa, useCli, useProxyAddr } from "@/lib/setup";
import { Row } from "./settings-ui";

/** Proxy address, certificate trust and shell variables. */
export function AgentSetup() {
  const desktop = isTauri();
  const { ca, refresh } = useCa();
  const [busy, setBusy] = useState(false);
  const cli = useCli();
  const addr = useProxyAddr();
  const env = proxyEnv(ca?.path ?? "~/.friction/friction-root-ca.pem", addr);

  return (
    <>
      <Row title="Proxy" detail="Point agents' HTTP and HTTPS proxy here.">
        <code className="font-mono text-xs select-text">{addr}</code>
      </Row>
      <Row
        title="Certificate"
        detail={
          !desktop
            ? "Desktop app only."
            : !ca
              ? "Checking…"
              : !ca.available
                ? `Unavailable: ${ca.error ?? "unknown error"}`
                : ca.trusted
                  ? "Trusted. Generated on this Mac, private key never leaves it."
                  : "Not trusted yet, so HTTPS requests through Friction will fail."
        }
      >
        {desktop && ca?.available && (
          <div className="flex gap-1.5">
            <Button variant="ghost" size="sm" className="text-xs" onClick={() => void revealCa()}>
              Show in Finder
            </Button>
            {!ca.trusted && (
              <Button
                size="sm"
                className="text-xs"
                disabled={busy}
                onClick={async () => {
                  setBusy(true);
                  await trustCertificate(refresh);
                  setBusy(false);
                }}
              >
                {busy ? "Waiting…" : "Trust"}
              </Button>
            )}
          </div>
        )}
      </Row>
      <Row
        title="Command-line tool"
        detail={
          cli.path ? (
            <>
              Installed at <code className="font-mono select-text">{cli.path}</code>. Use{" "}
              <code className="font-mono select-text">friction run -- your-agent</code>.
            </>
          ) : (
            "friction run -- your-agent sets everything up for one command."
          )
        }
      >
        {desktop && (
          <Button
            variant="outline"
            size="sm"
            className="text-xs"
            onClick={() => void cli.install()}
          >
            {cli.path ? "Reinstall" : "Install"}
          </Button>
        )}
      </Row>
      <div className="px-4 py-3">
        <div className="flex items-center justify-between gap-4">
          <div>
            <p className="text-[13px]">Shell variables</p>
            <p className="mt-0.5 text-xs text-muted-foreground">
              Node, Python and curl keep their own certificate lists, so set these too.
            </p>
          </div>
          <Button
            variant="ghost"
            size="sm"
            className="text-xs"
            onClick={() => void copyText(env, "shell variables")}
          >
            Copy
          </Button>
        </div>
        <pre className="mt-2 overflow-x-auto rounded-md border bg-background/60 p-2.5 font-mono text-[11px] leading-relaxed select-text">
          {env}
        </pre>
      </div>
    </>
  );
}
