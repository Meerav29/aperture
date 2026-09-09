# macOS validation handoff

Status: **pending real Mac execution**. The project owner has no Mac; this
checklist is for the friend who will clone and run the project. Windows results
do not establish macOS compatibility. A macOS CI build can catch compile/test
failures but cannot replace the signed-in application checks below.

Read [product vision](../product-vision.md), [current goals](../goals.md), and
[Windows evidence](README.md). Target local Claude Code and Codex sessions in
Terminal.app, VS Code extensions, and provider desktop coding apps. Ordinary
Claude Chat/Cowork, remote sessions, WSL, and containers are outside this test.

## 1. Prepare and record the exact revision

Install Node 20+, Rust stable, Git, and the
[Tauri macOS prerequisites](https://v2.tauri.app/start/prerequisites/).
Install/sign in to both providers' CLI, VS Code extension, and desktop apps.
Do not share credentials or copy provider configuration to the repository.

Run from Terminal.app:

```sh
git clone https://github.com/Meerav29/aperture.git
cd aperture
git rev-parse HEAD
sw_vers
uname -m
node --version
rustc --version
claude --version
codex --version
npm ci
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --locked
npm run tauri dev
```

If using an existing clone, first inspect git status and preserve local work
before updating. The latest Windows changes must be committed/pushed before a
clone can contain them; confirm the intended commit with the owner. Do not test
an old revision and label it as the latest implementation.

Record macOS version, architecture, commit, CLI versions, desktop app versions,
and extension versions. Build failures are useful results: retain the error and
stop claiming runtime success until fixed.

## 2. Establish passive discovery

With Aperture open, confirm separate provider integration health. Default file
roots are ~/.claude/projects and ~/.codex/sessions. Non-default roots can be
supplied through CLAUDE_CONFIG_DIR and CODEX_HOME in Aperture's environment.
An intentional custom configuration root is not automatically a product bug.
Record the effective roots with usernames redacted in shared evidence.

Use disposable local work, not important existing tasks. The human tester
starts and operates sessions in their original app; Aperture must not do so.
Test one session created before Aperture opens and one created afterwards.

## 3. Run each required combination

| Provider | Host | Procedure | Result |
|---|---|---|---|
| Claude Code | Terminal | Start interactive claude in Terminal.app and send a small prompt | Pending |
| Claude Code | VS Code | Use its local extension chat in a disposable folder | Pending |
| Claude Code | Desktop Code | Use the desktop Code experience with local execution | Pending |
| Codex | Terminal | Start interactive codex in Terminal.app and send a small prompt | Pending |
| Codex | VS Code | Use its local extension chat in a disposable folder | Pending |
| Codex | Desktop | Use the desktop app with local execution | Pending |

Non-interactive claude -p and codex exec may be extra checks, but cannot stand
in for the interactive terminal row. For every row separately, record:

- Discovery of the correct provider/session, working directory, and host label.
- Prompt -> working -> tool activity -> completed turn, with visible timings.
- One explicit question and one permission request if that host/version supports
  them. Respond in the original host and check attention clears accurately.
- Interruption and normal session close; unknown process liveness must remain
  unknown, not a fabricated end/idle state.
- Aperture closed during agent work, then restarted: agent work is unaffected;
  backfilled history is not falsely labeled live.
- Run one Claude session and one Codex session concurrently to check identities.

A transcript discovered from a host proves file discovery only. A historical
file or metadata label cannot mark all lifecycle or permission checks passed.
Usage limits or unavailable features mean the relevant test is pending/blocked.

## 4. Optional hook enrichment

Passive observation must work without installing anything. If the tested
revision offers explicit hook setup, follow the [helper setup report](windows-compatibility-followup.md)
and that revision's README, adapting the executable path to macOS, and record
whether hooks are enabled and supported by each installed provider version.
Preserve a local backup and compare provider settings before/after setup/removal;
only Aperture-owned entries may change. Do not share raw settings.

Check permission attention with hooks enabled, then with the integration
disabled/unavailable. Missing signals must be labeled unknown. Never enable
automatic approval, bypass permission checks, or add hooks that make decisions
merely to produce a passing observer test.

## 5. Navigation, restart, and display

Click Open folder and Reveal transcript for each host; record precisely whether
Finder opens the working directory or transcript-containing folder. Do not call
folder navigation exact-session focus. Check missing paths and OS-denied actions
produce understandable failures. Test ordinary window resize and display scaling.

Restart Aperture and sleep/wake the Mac during disposable activity. Check
history/health/freshness and ensure no agent work is started or stopped by
Aperture. Record limitations rather than changing expected results after a failure.

## 6. Return evidence

Create docs/validation/macos-results.md in a branch or send the owner a sanitized
Markdown report. Use this template for EACH host:

```text
Date/time/timezone:
Commit:
macOS version / architecture:
Provider / CLI version / app or extension version:
Host / local execution:
Passive or hooks enabled:
Effective root (redacted):
Disposable session ID (redacted consistently if needed):
Steps:
Discovery result:
Prompt/tool/finish result:
Permission/input request and resolution result:
Interruption/session-close result:
Restart/sleep-wake result:
Navigation action and actual result:
Build/test outputs:
Evidence references (sanitized screenshot or trace):
Known failures / pending checks:
```

Do not commit raw prompts, source code from unrelated repos, credentials,
full environment dumps, or unsanitized transcripts. Use fixtures containing only
fields needed to reproduce parser issues. Keep settings backups on your Mac.

Update SPIKE.md only for capabilities actually demonstrated; attach this result
file and exact evidence. Fixes require rerunning the affected tests on the Mac.
All six rows must have the required evidence before declaring macOS validated.
