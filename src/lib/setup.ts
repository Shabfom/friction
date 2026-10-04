import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { cliStatus, getCaInfo, getProxyStatus, installCa, installCli, type CaInfo } from "./tauri-ipc";

export function useProxyAddr() {
  const [addr, setAddr] = useState("127.0.0.1:8080");
  useEffect(() => {
    void getProxyStatus().then((s) => s?.addr && setAddr(s.addr));
  }, []);
  return addr;
}

export function proxyEnv(certPath: string, addr: string) {
  return [
    `export HTTPS_PROXY=http://${addr} HTTP_PROXY=http://${addr}`,
    `export NODE_EXTRA_CA_CERTS="${certPath}" REQUESTS_CA_BUNDLE="${certPath}" SSL_CERT_FILE="${certPath}"`,
  ].join("\n");
}

export async function copyText(text: string, what: string) {
  try {
    await navigator.clipboard.writeText(text);
    toast.success(`Copied ${what}`);
  } catch {
    toast.error("Couldn't copy. Select the text and copy it manually.");
  }
}

export function useCa() {
  const [ca, setCa] = useState<CaInfo | null>(null);
  const refresh = useCallback(() => void getCaInfo().then(setCa), []);
  useEffect(refresh, [refresh]);
  return { ca, refresh };
}

export async function trustCertificate(refresh: () => void) {
  const res = await installCa();
  if (res.ok) toast.success("Certificate trusted.");
  else toast.error(`Not trusted: ${res.error ?? "cancelled"}`);
  refresh();
}

export function useCli() {
  const [path, setPath] = useState<string | null>(null);
  const refresh = useCallback(
    () => void cliStatus().then((s) => setPath(s?.installedAt ?? null)),
    [],
  );
  useEffect(refresh, [refresh]);
  const install = useCallback(async () => {
    const res = await installCli();
    if (res.ok && res.path) {
      setPath(res.path);
      toast.success(
        res.path.includes("/.local/bin/")
          ? `Installed at ${res.path}. Add ~/.local/bin to your PATH.`
          : `Installed: ${res.path}`,
      );
    } else toast.error(`Couldn't install: ${res.error ?? "unknown error"}`);
  }, []);
  return { path, install };
}
