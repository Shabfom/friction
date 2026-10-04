# Friction: development notes

What Friction does and how to use it is in [README.md](README.md). This file
covers working on the code.

## Layout

- `src/` React UI (Vite, Tailwind). `src/lib/tauri-ipc.ts` and
  `src/lib/activity-ipc.ts` are the bridge to Rust.
- `src-tauri/src/` Rust backend:
  - `proxy.rs` the HTTP/HTTPS proxy, holds and decisions
  - `payments.rs` payment endpoint detection and amounts
  - `secrets.rs` secret detection and masking
  - `spend.rs`, `meter.rs` AI API usage parsing, prices, daily totals
  - `flight.rs` Activity log and HAR export
  - `rules_file.rs`, `policy.rs` rules.toml parsing and layering
  - `ca.rs` the per-machine certificate authority
  - `shell.rs` menu bar, Dock badge, app menu, open at login
- `cli/friction` the command-line companion (bundled into the app).

## Build

```sh
npm install
npm run tauri -- build --bundles app                               # release .app
npm run tauri -- build --target universal-apple-darwin --bundles dmg   # universal .dmg
```

The universal build needs both Rust targets:
`rustup target add aarch64-apple-darwin x86_64-apple-darwin`.

## Live development loop

`scripts/dev-watch.sh` rebuilds the app whenever source files change, installs
it to `/Applications/Friction.app` and relaunches it. Control files live in
`.friction-dev/` (`paused`, `rebuild`, `dmg`, `run-tests`).

## Tests

- `cargo test` in `src-tauri/` runs the unit tests.
- `scripts/proxy-test.sh` runs end-to-end checks against the running app.
