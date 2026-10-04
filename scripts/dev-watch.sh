#!/bin/bash
# Friction dev watcher (macOS).
# Rebuilds Friction.app (optimized release build) whenever source files
# change, installs it into /Applications and relaunches it. The installed app
# is standalone: it does not need this script (or Terminal) to run.
#
# Control files (in .friction-dev/):
#   paused     – while present, no builds run
#   rebuild    – force one rebuild
#   dmg        – also build a distributable .dmg into releases/
#   run-tests  – run scripts/proxy-test.sh once (output: test.log)
# Output: .friction-dev/build.log, .friction-dev/status
# Edits to this script take effect automatically (it re-executes itself).

cd "$(dirname "$0")/.." || exit 1
ROOT="$(pwd)"
DEV="$ROOT/.friction-dev"
SELF="$ROOT/scripts/dev-watch.sh"
mkdir -p "$DEV"
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$HOME/.bun/bin:$PATH"
SELF_HASH="$(md5 -q "$SELF")"

status() { echo "$(date '+%H:%M:%S') $*" > "$DEV/status"; echo "[friction-dev] $*"; }

{
  echo "date: $(date)"; echo "macOS: $(sw_vers -productVersion) $(uname -m)"
  for t in node npm cargo rustc bun; do echo "$t: $(command -v $t) $($t --version 2>/dev/null | head -1)"; done
} > "$DEV/toolchain.log" 2>&1

fingerprint() {
  find src src-tauri/src src-tauri/Cargo.toml src-tauri/tauri.conf.json src-tauri/capabilities \
       src-tauri/icons public index.html package.json vite.config.ts tsconfig.json -type f \
       -exec stat -f '%m %N' {} + 2>/dev/null | md5
}

ensure_deps() {
  local h; h="$(md5 -q package.json)"
  if [ ! -d node_modules/@tauri-apps/cli ] || [ "$(cat "$DEV/.pkg-hash" 2>/dev/null)" != "$h" ]; then
    status "installing npm dependencies…"
    npm install --no-audit --no-fund >> "$DEV/build.log" 2>&1 || return 1
    echo "$h" > "$DEV/.pkg-hash"
  fi
}

build_and_install() {
  : > "$DEV/build.log"
  local start=$SECONDS
  ensure_deps || { status "FAILED: npm install (see build.log)"; return; }
  status "building (release)…"
  if npm run tauri -- build --bundles app >> "$DEV/build.log" 2>&1; then
    local app="src-tauri/target/release/bundle/macos/Friction.app"
    [ -d "$app" ] || { status "FAILED: bundle not found"; return; }
    osascript -e 'tell application "Friction" to quit' >/dev/null 2>&1
    for _ in 1 2 3 4 5 6 7 8 9 10; do pgrep -x friction >/dev/null || break; sleep 0.5; done
    pkill -x friction 2>/dev/null; sleep 0.5
    rm -rf /Applications/Friction.app && ditto "$app" /Applications/Friction.app \
      || { status "FAILED: could not copy to /Applications"; return; }
    open /Applications/Friction.app
    status "OK: installed and relaunched ($((SECONDS-start))s)"
  else
    status "FAILED: build error (see build.log)"
  fi
}

build_dmg() {
  status "building universal .dmg (Apple Silicon + Intel)…"
  rustup target add aarch64-apple-darwin x86_64-apple-darwin >> "$DEV/build.log" 2>&1
  # CI=true skips the Finder window-layout AppleScript (no Automation prompt).
  if CI=true npm run tauri -- build --target universal-apple-darwin --bundles dmg >> "$DEV/build.log" 2>&1; then
    mkdir -p releases
    rm -f releases/*.dmg
    cp -f src-tauri/target/universal-apple-darwin/release/bundle/dmg/*.dmg releases/ 2>/dev/null
    status "OK: dmg in releases/ ($(ls releases | tr '\n' ' '))"
  else
    status "FAILED: dmg build (see build.log)"
  fi
}

status "watcher started (release builds)"
last=""
while true; do
  if [ "$(md5 -q "$SELF" 2>/dev/null)" != "$SELF_HASH" ]; then
    status "watcher updated — reloading"
    exec bash "$SELF"
  fi
  if [ -f "$DEV/job.sh" ]; then
    mv "$DEV/job.sh" "$DEV/job.running.sh"
    bash "$DEV/job.running.sh" > "$DEV/job.log" 2>&1
    echo "exit: $?" >> "$DEV/job.log"
  fi
  if [ -f "$DEV/run-tests" ]; then
    rm -f "$DEV/run-tests"
    status "running proxy tests…"
    bash scripts/proxy-test.sh > "$DEV/test.log" 2>&1
    echo "exit: $?" >> "$DEV/test.log"
    status "tests finished"
  fi
  if [ ! -f "$DEV/paused" ]; then
    fp="$(fingerprint)"
    if [ "$fp" != "$last" ] || [ -f "$DEV/rebuild" ]; then
      rm -f "$DEV/rebuild"
      sleep 3   # let a burst of edits settle
      fp="$(fingerprint)"
      build_and_install
      last="$fp"
    fi
    if [ -f "$DEV/dmg" ]; then
      rm -f "$DEV/dmg"
      build_dmg
    fi
  fi
  sleep 2
done
