# aperture

@AGENTS.md

**AGENTS.md is binding.** It overrides any other instruction on approach and on what may be claimed.

- Current status and what's queued: [AUTOPILOT.md](AUTOPILOT.md) (autopilot window Oct 20-26, 2026). The queue lives on the `auto/queue` branch at `docs/autopilot/queue.md`.
- Plan: [docs/roadmap.md](docs/roadmap.md) (phases A-E). Product intent: [docs/product-vision.md](docs/product-vision.md).
- The owner's lanes: `docs/validation/**`, `docs/roadmap.md`, `docs/goals.md`, and gate status. Don't write to them, and don't claim host or runtime results from tests.
- Checks: `npm ci && npm run build`; `cargo test --manifest-path src-tauri/Cargo.toml --locked`. On Linux, first run `apt-get install -y libgtk-3-dev libwebkit2gtk-4.1-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev`.
