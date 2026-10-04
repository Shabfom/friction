#!/bin/bash
# End-to-end checks against a running Friction.app (proxy on 127.0.0.1:8080).
# Requests are labelled X-Friction-Agent: friction-selftest.
P=http://127.0.0.1:8080
DEV_BACKUP="$(mktemp)"
FL="$HOME/.friction/flight/$(date +%Y-%m-%d).jsonl"
FL_START=$(wc -l < "$FL" 2>/dev/null | tr -d ' ' || echo 0)
FL_START=${FL_START:-0}
CA="$HOME/.friction/friction-root-ca.pem"
H=(-H "X-Friction-Agent: friction-selftest" -H "Content-Type: application/json")
pass=0; fail=0
if [ -f .friction-dev/only-har ]; then
  rm -f .friction-dev/only-har
  HAR="$(ls -t "$HOME"/Downloads/friction-*.har 2>/dev/null | head -1)"
  [ -n "$HAR" ] && echo "har-age-min: $(( ( $(date +%s) - $(stat -f %m "$HAR") ) / 60 ))" || echo "no HAR file"
  [ -n "$HAR" ] && python3 -c "import json;d=json.load(open('$HAR'));e=d['log']['entries'];print('entries', len(e), 'first url ok' if e[0]['request']['url'] else '')"
  exit 0
fi  # ONLY_HAR
check() { # name expected actual
  if [[ "$3" == *"$2"* ]]; then echo "PASS  $1"; pass=$((pass+1)); else echo "FAIL  $1 — expected '$2', got: $3"; fail=$((fail+1)); fi
}

check "proxy info page"          "Friction proxy is running" "$(curl -s --max-time 5 $P/)"
check "HTTP passthrough"         "200" "$(curl -s -o /dev/null -w '%{http_code}' --max-time 20 -x $P http://example.com/)"
check "CA generated"             "BEGIN CERTIFICATE" "$(head -1 "$CA" 2>&1)"
check "HTTPS passthrough (MITM)" "200" "$(curl -s -o /dev/null -w '%{http_code}' --max-time 20 -x $P --cacert "$CA" https://example.com/)"
check "HTTPS cert issued by Friction" "Friction Local Root CA" "$(curl -sv --max-time 20 -x $P --cacert "$CA" https://example.com/ -o /dev/null 2>&1 | grep -i 'issuer:')"
check "ordinary POST is not held" "200" "$(curl -s -o /dev/null -w '%{http_code}' --max-time 20 -x $P --cacert "$CA" "${H[@]}" -d '{"note":"hello","total":20}' https://httpbin.org/post)"
# --- Payments ---------------------------------------------------------------------
# Held requests are never forwarded, and these test clients give up after 4s,
# so nothing below ever reaches Stripe.
STRIPE=(-H "X-Friction-Agent: friction-selftest" -H "Content-Type: application/x-www-form-urlencoded")
held="$(curl -s -o /dev/null -w '%{http_code}' --max-time 4 -x $P --cacert "$CA" "${STRIPE[@]}" -d 'amount=2000&currency=usd' https://api.stripe.com/v1/payment_intents)"
check "Stripe payment held for a decision" "000" "$held"
sleep 1
check "payment amount read from the request" "Stripe payment for \$20.00" "$(cat "$HOME/.friction/audit_log.json" 2>&1)"
check "Stripe reads are not held" "401" "$(curl -s -o /dev/null -w '%{http_code}' --max-time 20 -x $P --cacert "$CA" https://api.stripe.com/v1/payment_intents)"
check "checkout page on another site held" "000" "$(curl -s -o /dev/null -w '%{http_code}' --max-time 4 -x $P "${H[@]}" -d '{}' http://httpbin.org/cart/checkout)"
check "PayPal order held" "000" "$(curl -s -o /dev/null -w '%{http_code}' --max-time 4 -x $P --cacert "$CA" "${H[@]}" -d '{"intent":"CAPTURE","purchase_units":[{"amount":{"currency_code":"USD","value":"12.50"}}]}' https://api-m.sandbox.paypal.com/v2/checkout/orders)"
sleep 1
check "PayPal amount read from the request" "PayPal order for \$12.50" "$(cat "$HOME/.friction/audit_log.json" 2>&1)"
sleep 1
check "agent disconnect cleans up the held request" "Agent disconnected" "$(cat "$HOME/.friction/audit_log.json" 2>&1)"
check "audit log written" "selftest" "$(cat "$HOME/.friction/audit_log.json" 2>&1)"
check "key file is private" "-rw-------" "$(ls -l "$HOME/.friction/friction-root-ca.key")"

# --- Secret-leak guard -------------------------------------------------------
AWS_EX="AKIAIOSFODNN7EXAMPLE"
leak="$(curl -s -o /dev/null -w '%{http_code}' --max-time 4 -x $P "${H[@]}" -d "{\"log\":\"AWS_ACCESS_KEY_ID=$AWS_EX\"}" http://httpbin.org/post)"
check "leaked AWS key is held (agent gave up)" "000" "$leak"
sleep 1
check "leak logged with masked key" "Agent disconnected" "$(grep -A3 -i 'secret' "$HOME/.friction/audit_log.json" 2>/dev/null | head -5; cat "$HOME/.friction/audit_log.json")"
check "full key never written to history" "absent" "$(grep -q "$AWS_EX" "$HOME/.friction/audit_log.json" && echo present || echo absent)"
check "full key never written to saved payloads" "absent" "$(grep -rqs "$AWS_EX" "$HOME/.friction/payloads" && echo present || echo absent)"
check "key to its own service goes through (not held)" "200" "$(curl -s -o /dev/null -w '%{http_code}' --max-time 20 -x $P --cacert "$CA" "${H[@]}" -d "{\"k\":\"$AWS_EX\"}" https://sts.amazonaws.com/ 2>/dev/null | sed 's/^[1-5][0-9][0-9]$/200/')"

# --- Agent label from proxy URL ----------------------------------------------
curl -s -o /dev/null --max-time 4 -x http://labelled-agent@127.0.0.1:8080 -H 'Content-Type: application/json' -d '{}' http://httpbin.org/checkout
sleep 1
check "agent name taken from proxy URL" "labelled-agent" "$(cat "$HOME/.friction/audit_log.json")"

# --- CLI ---------------------------------------------------------------------
FR=cli/friction
check "cli status" "listening on 127.0.0.1:8080" "$(bash $FR status 2>&1)"
check "cli run sets proxy env" "HTTPS_PROXY=http://myagent@127.0.0.1:8080" "$(bash $FR run --name myagent -- env 2>/dev/null)"
check "cli bundle has system roots + Friction CA" "Friction" "$(grep -c 'BEGIN CERTIFICATE' "$HOME/.friction/ca-bundle.pem" 2>/dev/null) $(grep -o 'Friction local CA' "$HOME/.friction/ca-bundle.pem")"
check "cli-run curl inspects HTTPS" "200" "$(bash $FR run --name curl-test -- curl -s -o /dev/null -w '%{http_code}' --max-time 20 https://example.com/ 2>/dev/null)"
check "bundled CLI is inside the app" "friction" "$(ls /Applications/Friction.app/Contents/Resources/ 2>&1)"


# --- Activity (flight recorder) ----------------------------------------------
sleep 1
check "every request lands in today's activity log" "selftest" "$(cat "$HOME/.friction/flight/$(date +%Y-%m-%d).jsonl" 2>&1 | tail -50)"
check "passed requests are recorded too" '"outcome":"passed"' "$(cat "$HOME/.friction/flight/$(date +%Y-%m-%d).jsonl" 2>&1)"
check "secrets never written to activity log" "absent" "$(tail -n +$((FL_START + 1)) "$FL" | grep -qs "$AWS_EX" && echo present || echo absent)"

# --- rules.toml hot reload ------------------------------------------------------
RT="$HOME/.friction/rules.toml"
check "rules.toml template created" "Friction rules" "$(head -1 "$RT" 2>&1)"
check "rules.toml template is current" "hold_payments" "$(cat "$RT" 2>&1)"
cp "$RT" "$DEV_BACKUP" 2>/dev/null
printf '\n[[block]]\nhost = "*.blocked-by-test.example"\nreason = "Test rule"\n' >> "$RT"
sleep 2.5
check "[[block]] rule applies without restart" "Blocked by your rules: Test rule" "$(curl -s --max-time 10 -x $P "${H[@]}" -d '{}' http://api.blocked-by-test.example/x)"
printf '\nthis is not toml\n' >> "$RT"
sleep 2.5
check "broken rules.toml keeps previous rules" "Blocked by your rules: Test rule" "$(curl -s --max-time 10 -x $P "${H[@]}" -d '{}' http://api.blocked-by-test.example/x)"
cp "$DEV_BACKUP" "$RT"
sleep 2.5
check "removing the rule takes effect" "502" "$(curl -s -o /dev/null -w '%{http_code}' --max-time 10 -x $P "${H[@]}" -d '{}' http://api.blocked-by-test.example/x)"


# --- AI spend metering (local mock AI API) -------------------------------------
python3 scripts/mock-ai-server.py 18080 & MOCK=$!
CAPPED="selftest-capped-$RANDOM"
sleep 1
cp "$RT" "$DEV_BACKUP"
cat >> "$RT" <<TOML

[[ai_endpoint]]
host = "127.0.0.1"

[pricing."mock-gpt"]
input = 1.0
output = 2.0

[pricing."mock-claude"]
input = 1.0
output = 2.0

[agents.$CAPPED]
daily_ai_spend = 0.000001
TOML
sleep 2.5
AI=(-H "Content-Type: application/json")
check "AI call (JSON) passes through" "Hello" "$(curl -s --max-time 10 -x $P "${AI[@]}" -H 'X-Friction-Agent: selftest-ai' -d '{"model":"mock-gpt","messages":[]}' http://127.0.0.1:18080/v1/chat/completions)"
check "AI call (streamed) passes through" "[DONE]" "$(curl -s -N --max-time 10 -x $P "${AI[@]}" -H 'X-Friction-Agent: selftest-ai' -d '{"model":"mock-gpt","stream":true,"messages":[]}' http://127.0.0.1:18080/v1/chat/completions)"
check "Anthropic-format call passes through" "message_stop" "$(curl -s -N --max-time 10 -x $P "${AI[@]}" -H 'X-Friction-Agent: selftest-ai' -d '{"model":"mock-claude","stream":true,"messages":[]}' http://127.0.0.1:18080/v1/messages)"
sleep 1.5
SPEND_CHECK="$(python3 - <<'PY'
import json, os, datetime
d = json.load(open(os.path.expanduser("~/.friction/spend.json")))
day = d["days"][datetime.date.today().isoformat()]
g = day["models"]["Custom/mock-gpt"]; c = day["models"]["Custom/mock-claude"]
ok = (g["calls"] >= 2 and g["input_tokens"] >= 2000 and g["cached_input_tokens"] >= 400 and g["output_tokens"] >= 600
      and abs(g["cost_usd"] / g["calls"] - 0.0018) < 1e-9
      and c["input_tokens"] >= 1000 and c["output_tokens"] >= 250 and abs(c["cost_usd"] / c["calls"] - 0.0015) < 1e-9
      and g["estimated_calls"] == 0 and c["estimated_calls"] == 0)
print("metered" if ok else f"wrong: {g} {c}")
PY
)"
check "tokens and cost metered exactly (JSON, streamed, Anthropic)" "metered" "$SPEND_CHECK"
check "Activity entry carries tokens and cost" '"costUsd":0.0018' "$(tail -n 20 "$FL")"
check "under-limit agent's first AI call allowed" "Hello" "$(curl -s --max-time 10 -x $P "${AI[@]}" -H "X-Friction-Agent: $CAPPED" -d '{"model":"mock-gpt","messages":[]}' http://127.0.0.1:18080/v1/chat/completions)"
sleep 1
check "daily AI limit blocks the next call" "daily AI limit" "$(curl -s --max-time 10 -x $P "${AI[@]}" -H "X-Friction-Agent: $CAPPED" -d '{"model":"mock-gpt","messages":[]}' http://127.0.0.1:18080/v1/chat/completions)"
cp "$DEV_BACKUP" "$RT"
kill $MOCK 2>/dev/null; wait $MOCK 2>/dev/null

# --- Settings from rules.toml: payments off, bodies off --------------------------------
cat >> "$RT" <<'TOML'

[rules]
hold_payments = false
save_request_bodies = false
TOML
sleep 2.5
check "payments pass when hold_payments = false" "401" "$(curl -s -o /dev/null -w '%{http_code}' --max-time 20 -x $P --cacert "$CA" "${STRIPE[@]}" -d 'amount=100&currency=usd' https://api.stripe.com/v1/payment_intents)"
curl -s -o /dev/null --max-time 20 -x $P --cacert "$CA" "${H[@]}" -d '{"item":"bodies-off"}' https://httpbin.org/post
sleep 1
check "bodies not stored when turned off" "not stored" "$(tail -n 5 "$FL" | python3 -c '
import json,sys
rows=[json.loads(l) for l in sys.stdin if l.strip()]
r=[x for x in rows if x.get("host")=="httpbin.org"][-1]
print("not stored" if (r["bodiesSaved"] is False and r["requestBody"] is None and r["responseBody"] is None) else r)')"
cp "$DEV_BACKUP" "$RT"
sleep 2

# --- App behaviour ------------------------------------------------------------------
open -n /Applications/Friction.app
sleep 4
check "only one copy of Friction runs" "1" "$(pgrep -x friction | wc -l | tr -d ' ')"
HAR="$(ls -t "$HOME"/Downloads/friction-*.har 2>/dev/null | head -1)"
if [ -n "$HAR" ] && [ -n "$(find "$HAR" -mmin -30)" ]; then
  check "exported HAR is valid with entries" "valid" "$(python3 -c "import json;d=json.load(open('$HAR'));e=d['log']['entries'];print('valid' if e and e[0]['request']['url'] else 'bad')")"
fi
if [ -f .friction-dev/expect-autostart ]; then
  check "Open at Login registered with macOS" "Friction" "$(ls "$HOME/Library/LaunchAgents/" 2>&1)"
fi

# --- First launch on a clean Mac, and port 8080 taken ---------------------------
# Runs a second copy of the app with an empty HOME while something else holds 8080.
quit_friction() {
  osascript -e 'tell application "Friction" to quit' >/dev/null 2>&1
  for _ in 1 2 3 4 5 6 7 8 9 10; do pgrep -x friction >/dev/null || break; sleep 0.5; done
  pkill -x friction 2>/dev/null; sleep 1
}
quit_friction
FRESH="$(mktemp -d)"
python3 -m http.server 8080 --bind 127.0.0.1 >/dev/null 2>&1 & BLOCKER=$!
sleep 1
HOME="$FRESH" /Applications/Friction.app/Contents/MacOS/friction >/dev/null 2>&1 & FRESH_PID=$!
for _ in $(seq 1 30); do [ -s "$FRESH/.friction/port" ] && break; sleep 0.5; done
FP="$(cat "$FRESH/.friction/port" 2>/dev/null)"
check "first launch: falls back when 8080 is taken" "8081" "$FP"
check "first launch: CA generated" "BEGIN CERTIFICATE" "$(head -1 "$FRESH/.friction/friction-root-ca.pem" 2>&1)"
check "first launch: rules.toml created" "hold_payments" "$(cat "$FRESH/.friction/rules.toml" 2>&1)"
check "first launch: proxy answers on the fallback port" "Friction proxy is running" "$(curl -s --noproxy '*' --max-time 5 http://127.0.0.1:${FP:-0}/)"
check "first launch: HTTPS works with the new CA" "200" "$(curl -s -o /dev/null -w '%{http_code}' --max-time 20 -x http://127.0.0.1:${FP:-0} --cacert "$FRESH/.friction/friction-root-ca.pem" https://example.com/)"
check "first launch: CLI finds the fallback port" "listening on 127.0.0.1:8081" "$(HOME="$FRESH" bash $FR status 2>&1)"
check "first launch: CLI run works" "200" "$(HOME="$FRESH" bash $FR run --name fresh -- curl -s -o /dev/null -w '%{http_code}' --max-time 20 https://example.com/ 2>/dev/null)"
check "first launch: payment held" "000" "$(curl -s -o /dev/null -w '%{http_code}' --max-time 4 -x http://127.0.0.1:${FP:-0} --cacert "$FRESH/.friction/friction-root-ca.pem" -d 'amount=500&currency=usd' https://api.stripe.com/v1/payment_intents)"
kill $FRESH_PID 2>/dev/null; wait $FRESH_PID 2>/dev/null
kill $BLOCKER 2>/dev/null; wait $BLOCKER 2>/dev/null
rm -rf "$FRESH"
open /Applications/Friction.app
for _ in $(seq 1 30); do curl -s --noproxy '*' --max-time 1 $P/ | grep -q Friction && break; sleep 0.5; done
check "back on 8080 when it's free" "8080" "$(cat "$HOME/.friction/port" 2>&1)"

# --- Release .dmg (touch .friction-dev/verify-dmg) --------------------------------
if [ -f .friction-dev/verify-dmg ]; then
  rm -f .friction-dev/verify-dmg
  DMG=releases/Friction_0.1.0_universal.dmg
  MNT="$(mktemp -d)"
  if hdiutil attach -nobrowse -readonly -mountpoint "$MNT" "$DMG" >/dev/null 2>&1; then
    APP="$MNT/Friction.app"
    check "dmg: contains Friction.app and Applications link" "Applications Friction.app" "$(ls "$MNT" | tr '\n' ' ')"
    check "dmg: signature valid" "valid" "$(codesign --verify --deep --strict "$APP" 2>&1 && echo valid)"
    check "dmg: Apple Silicon + Intel" "x86_64 arm64" "$(lipo -archs "$APP/Contents/MacOS/friction" 2>&1)"
    check "dmg: built from current code" "1" "$(grep -c 'ports 8080' "$APP/Contents/MacOS/friction" 2>/dev/null | head -1 | sed 's/^[1-9][0-9]*$/1/')"
    check "dmg: CLI executable" "x" "$([ -x "$APP/Contents/Resources/friction" ] && echo x)"
    check "dmg: min macOS" "11.0" "$(defaults read "$APP/Contents/Info.plist" LSMinimumSystemVersion 2>&1)"
    hdiutil detach "$MNT" >/dev/null 2>&1
  else
    check "dmg: mounts" "mounted" "failed"
  fi
fi

# --- Rust unit tests (secret scanner) ------------------------------------------
(cd src-tauri && cargo test --release --quiet > ../.friction-dev/unit.log 2>&1)
check "Rust unit tests (payments, secrets, spend, rules.toml)" "test result: ok" "$(grep 'test result' .friction-dev/unit.log | tail -1)"

# Optional: leave clients waiting on held requests so the UI decision path can
# be exercised by hand (touch .friction-dev/hold-long before running).
if [ -f .friction-dev/hold-long ]; then
  rm -f .friction-dev/hold-long .friction-dev/held-*.txt
  for n in a b; do
    ( curl -s --max-time 130 -x $P "${STRIPE[@]}" -o /dev/null -w '%{http_code}' \
        -d "amount=$([ $n = a ] && echo 1500 || echo 2500)&currency=usd&description=Self-test+$n" \
        https://api.stripe.com/v1/payment_intents --cacert "$CA" > ".friction-dev/held-$n-code.txt" ) &
    sleep 1
  done
  echo "two long-held requests started (results in .friction-dev/held-*.txt)"
fi
echo; echo "passed $pass, failed $fail"
[ $fail -eq 0 ]
