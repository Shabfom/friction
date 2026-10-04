import { useEffect, useState } from "react";
import { Switch } from "@/components/ui/switch";
import { Button } from "@/components/ui/button";
import { AgentSetup } from "./agent-setup";
import { DEFAULT_RULES, RULE_GROUPS, type RuleKey, type Rules } from "@/lib/friction-data";
import { requestNotificationPermission, sendSystemNotification } from "@/lib/notifications";
import {
  getAutostart,
  getRulesFileStatus,
  onAutostartChanged,
  onRulesChanged,
  openRulesFile,
  revealRulesFile,
  setAutostart,
  type RulesFileStatus,
} from "@/lib/tauri-ipc";
import { Row, Section } from "./settings-ui";
import { toast } from "sonner";

export function RulesView({
  desktop,
  rules,
  fileOverrides,
  onToggle,
  onClearHistory,
  onRestoreRules,
}: {
  desktop: boolean;
  rules: Rules;
  fileOverrides: string[];
  onToggle: (key: RuleKey) => void;
  onClearHistory: () => Promise<void>;
  onRestoreRules: () => void;
}) {
  const safeRules = rules || DEFAULT_RULES;
  const [notif, setNotif] = useState<string>("checking");
  const [launchAtLogin, setLaunchAtLogin] = useState(false);
  const [rulesFile, setRulesFile] = useState<RulesFileStatus | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  useEffect(() => {
    if (!confirmClear) return;
    const t = window.setTimeout(() => setConfirmClear(false), 3000);
    return () => window.clearTimeout(t);
  }, [confirmClear]);
  const locked = (key: string) => fileOverrides.includes(key);

  useEffect(() => {
    if (!desktop) return;
    void getRulesFileStatus().then(setRulesFile);
    const unlisten = onRulesChanged(setRulesFile);
    return () => void unlisten.then((fn) => fn());
  }, [desktop]);

  useEffect(() => void requestNotificationPermission().then(setNotif), []);
  useEffect(() => {
    if (!desktop) return;
    void getAutostart().then((v) => setLaunchAtLogin(!!v));
    const unlisten = onAutostartChanged(setLaunchAtLogin);
    return () => void unlisten.then((fn) => fn());
  }, [desktop]);

  const toggle = (key: RuleKey) => (
    <Switch
      checked={Boolean(safeRules[key])}
      onCheckedChange={() => onToggle(key)}
      disabled={locked(key)}
      title={locked(key) ? "Set in rules.toml" : undefined}
      className="data-[state=checked]:bg-safe"
    />
  );

  return (
    <div className="flex h-full flex-col">
      <header className="flex h-12 shrink-0 items-center border-b border-border/70 px-5">
        <h1 className="text-[13px] font-semibold">Rules</h1>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto px-5 py-5">
        <div className="mx-auto max-w-2xl space-y-6">
          {RULE_GROUPS.map((group) => (
            <Section key={group.group} title={group.group}>
              {group.rules.map((rule) => (
                <Row
                  key={rule.key}
                  title={rule.label}
                  detail={locked(rule.key) ? `${rule.detail} Set in rules.toml.` : rule.detail}
                >
                  {toggle(rule.key)}
                </Row>
              ))}
            </Section>
          ))}

          {desktop && (
            <Section title="Rules file">
              <Row
                title="rules.toml"
                detail={
                  !rulesFile ? (
                    "Checking…"
                  ) : rulesFile.ok ? (
                    <>
                      {rulesFile.customRules > 0
                        ? `${rulesFile.customRules} custom rule${rulesFile.customRules === 1 ? "" : "s"}. `
                        : ""}
                      {rulesFile.overrides.length > 0
                        ? `Overrides ${rulesFile.overrides.length} setting${rulesFile.overrides.length === 1 ? "" : "s"} here. `
                        : ""}
                      Block, hold or allow by host and path, set per-agent limits and model prices.
                      Changes apply when you save.
                    </>
                  ) : (
                    <span className="text-threat">
                      {rulesFile.error}. Using your previous rules until it&apos;s fixed.
                    </span>
                  )
                }
              >
                <div className="flex gap-1.5">
                  <Button
                    variant="ghost"
                    size="sm"
                    className="text-xs"
                    onClick={() => void revealRulesFile()}
                  >
                    Show in Finder
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    className="text-xs"
                    onClick={() => void openRulesFile()}
                  >
                    Edit
                  </Button>
                </div>
              </Row>
            </Section>
          )}

          <Section title="Connection">
            <AgentSetup />
          </Section>

          <Section title="App">
            {desktop && (
              <Row
                title="Open at login"
                detail="Starts in the menu bar. Closing the window doesn't stop Friction; quit from the menu bar icon."
              >
                <Switch
                  checked={launchAtLogin}
                  onCheckedChange={(v) =>
                    void setAutostart(v).then((now) => setLaunchAtLogin(!!now))
                  }
                  className="data-[state=checked]:bg-safe"
                />
              </Row>
            )}
            <Row
              title="Notifications"
              detail={
                notif === "granted"
                  ? "You'll get an alert when something is held or blocked."
                  : "Off. Turn on to hear about held requests while Friction is in the background."
              }
            >
              {notif === "granted" ? (
                <Button
                  variant="ghost"
                  size="sm"
                  className="text-xs"
                  onClick={async () => {
                    const ok = await sendSystemNotification(
                      "Friction",
                      "Notifications are working.",
                    );
                    if (!ok)
                      toast.error("Couldn't show it. Check System Settings › Notifications.");
                  }}
                >
                  Send test
                </Button>
              ) : (
                <Button
                  variant="outline"
                  size="sm"
                  className="text-xs"
                  onClick={async () => {
                    const res = await requestNotificationPermission();
                    setNotif(res);
                    if (res !== "granted")
                      toast.error("Turn on Friction in System Settings › Notifications.");
                  }}
                >
                  Turn on
                </Button>
              )}
            </Row>
            <Row
              title="Clear history"
              detail={
                desktop
                  ? "Deletes decisions, activity and AI spend history from this Mac. Rules and settings stay."
                  : "Restores the sample requests and clears decisions."
              }
            >
              <Button
                variant="ghost"
                size="sm"
                className="text-xs text-threat"
                onClick={async () => {
                  if (!confirmClear) {
                    setConfirmClear(true);
                    return;
                  }
                  setConfirmClear(false);
                  await onClearHistory();
                  toast.success("History cleared.");
                }}
              >
                {confirmClear ? "Click again to clear" : "Clear"}
              </Button>
            </Row>
            <Row
              title="Restore default rules"
              detail="Turns the switches above back to their defaults. Your AI spend limit and rules.toml are untouched."
            >
              <Button
                variant="ghost"
                size="sm"
                className="text-xs"
                onClick={() => {
                  onRestoreRules();
                  toast.success("Default rules restored.");
                }}
              >
                Restore
              </Button>
            </Row>
          </Section>
        </div>
      </div>
    </div>
  );
}
