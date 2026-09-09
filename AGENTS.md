# Working on Aperture

Read [docs/product-vision.md](docs/product-vision.md) first. It is the central
source of truth for product intent across sessions. Then read
[docs/goals.md](docs/goals.md), the relevant requirements/specification section,
and the latest validation report before selecting work.

- Preserve both Claude Code and Codex as first-class providers, externally
  created sessions, local Windows/macOS support, and monitor-and-jump scope.
- Inspect current code and Git state. A chat statement, merged PR, or passing
  unit suite does not establish complete real-host compatibility.
- Distinguish historical discovery, recent observations, stale state, unknown
  attention, and process liveness. Never turn missing evidence into success.
- Keep passive discovery available. Optional integrations must be explicitly
  configured, preserve unrelated settings, and never make agent decisions.
- The owner has no Mac. Prepare the documented friend handoff and continue
  independent Windows work; keep Mac runtime validation pending until performed.
- Update affected documentation and evidence when implementation changes.
  Describe current behavior separately from future requirements.
- Preserve unrelated work and coordinate file ownership when agents share a
  checkout. Do not silently switch, reset, or discard another task's work.

Useful checks: npm run build (includes TypeScript), cargo test --manifest-path
src-tauri/Cargo.toml --locked, and the relevant host-validation checklist.
Do not launch or control real user agent sessions to manufacture passing tests.
