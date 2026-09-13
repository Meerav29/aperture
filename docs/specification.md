# Aperture detailed specification and implementation phases

Status: target implementation contract, reviewed 2026-09-07. Detailed contracts
below are planned unless the current-state table or a dated validation report
explicitly proves implementation. No whole phase is complete yet.

[Product vision](product-vision.md) is the central intent source.
[Requirements](requirements.md) define acceptance; [goals](goals.md) tracks current
assignments. [Phase 0](../SPIKE.md) records compatibility coverage and limitations.
Revise implementation choices when evidence changes; preserve the product scope.

## 1. Current state and design delta

Retain Tauri 2, Rust, React 18, and TypeScript. The implemented baseline is passive
file observation for both providers, not the original HTTP/hook spike.

| Area | Audited baseline at 9df871d | Remaining target |
|---|---|---|
| Ingestion | Debounced filesystem watching plus 5s reconcile, bounded incremental JSONL reads for both providers | Optional attention enrichment |
| Identity | Provider-qualified IDs in passive adapters | Strong shared types, child identity and concurrent request tracking |
| Configuration | Passive roots via environment; no automatic settings writes | Safe explicit enrichment setup; eventual per-provider repair UI |
| Status | Recent/stale/history-only; process liveness unknown | Session-specific reconciliation and trustworthy attention |
| History | SQLite summaries and recoverable cursors; restart loads stale/history_only then reconciles | Retention/migration evidence at scale; git identity |
| Git | repo_root still unresolved; UI groups by cwd | Real repository/worktree identity |
| Navigation | Open working folder or transcript-containing folder | Validated destinations, errors, structured outcomes and optional exact focus |
| UI | Both providers, attention, freshness, health; revised subscription cleanup | Filters/details, accessibility, notifications and daily-use polish |
| Verification | Windows file-growth evidence and 14 Rust tests at consolidation | Live per-host attention evidence, Mac runs and release gates |

The current Windows follow-up is recorded in
[windows-compatibility-followup.md](validation/windows-compatibility-followup.md).
Use that report for subsequent changes and test counts. The baseline above
does not claim that this follow-up is complete.

The legacy hook installer and HTTP listener have been removed from the tree
(they were dormant, never registered by the baseline desktop, and carried an
unsafe settings parser/group removal and raw-ID reducer). The `aperture-hook`
CLI plus the file-inbox path (`hook_bridge.rs`/`hook_payload.rs`) is the
supported enrichment mechanism; any future automated settings-writing setup
must meet the configuration-safety contract below before being added, not
reuse the removed code. The passive implementation avoids provider settings
writes; that is not proof an automated installer would be safe. Historical
scans do not prove liveness. Counts and title fixes in the legacy transcript
reader do not automatically apply to the passive UI. Some prior hook-only and
host-field assumptions were too broad; verify actual installed-version
payloads.

The owner has no Mac. A friend will execute the
[Mac handoff](validation/macos-checklist.md). Windows implementation and test
preparation can continue while this is pending. macOS CI may check builds/tests,
but real host validation remains a cross-platform phase/release gate.

## 2. Architecture and ownership

Implemented baseline: provider discovery -> incremental file adapter -> shared
in-memory store -> Tauri snapshot -> desktop dashboard.

Current enrichment direction: explicitly configured provider hook -> small
observational helper -> sanitized local Aperture inbox -> shared reducer.
Passive discovery remains available with no hooks. Do not treat file discovery,
optional hook coverage, and verified process liveness as the same capability.

Production target: durable storage and incremental reconciliation behind the
same desktop contract, plus read-only Git identity, verified navigation, and
diagnostics. Planned HTTP transport below is an alternative, not a prerequisite
for the local inbox or passive collector.

Use one local observer owned by the desktop process. Tray/menu-bar behavior and
single-instance enforcement remain planned: hide on window close after explaining
it during onboarding; explicit Quit stops collection. Do not add a privileged
service. None of these features should own the agent processes.

### Provider adapters

Implement Claude Code and Codex as separate modules. Each owns:

- Effective configuration-root discovery and supported-version diagnostics.
- Hook configuration install/check/remove and external payload parsing.
- Historical discovery, transcript schema versions, and usage interpretation.
- Event-to-state translation and any verified provider-specific navigation.

The shared reducer must never switch on raw provider hook names. Use explicit
provider values claude_code and codex, independent of model names or API vendors.
Keep extension points small; a general plugin SDK is outside this release.

For the Windows follow-up, add a small Rust helper that reads hook JSON,
retains only observation metadata, writes to an Aperture-owned local inbox,
emits no decision output, and exits successfully on collection failure.
Configuration is printed for explicit manual merging; it must not replace
existing provider settings. A future packaged installer must satisfy the
configuration-safety contract below before it becomes an enabled feature.
Use an absolute executable path with provider-supported argument
encoding. Validate paths containing spaces, Unicode, and shell metacharacters.
Do not assume the immediate parent PID is the agent; resolve ancestry and record
unknown when identity cannot be proved. Never collect the full environment or
raw process command lines for telemetry.

Direct HTTP hooks can later replace forwarding where both payload coverage and
host attribution are adequate. They are not a prerequisite. Codex app-server
observation is an optional adapter experiment, not a machine-wide discovery
assumption and not a reason to resume an external session.

### Local ingestion

The current enrichment uses an Aperture-owned inbox with bounded record/file
handling. Never copy prompt text, tool inputs/outputs, reasoning, credentials, or
the full hook payload into it. Ignore malformed/oversized records, do not replay
old events as current activity, and report unavailable observation honestly.
See the Windows follow-up for implemented paths, limits, commands, and evidence.

The following HTTP design is a **future alternative** if justified by measured
latency or integration needs. It is not currently required or exposed.

Use versioned routes POST /v1/events/claude-code and POST /v1/events/codex.
Keep GET /health free of session content. Migrate the prototype /hook registration
during explicit integration repair; do not leave an unauthenticated legacy route
in the production build.

Bind only to loopback. Authenticate with an installation token stored in
user-private app configuration and read by the forwarder; do not put it in
diagnostics. Reject incorrect authentication, invalid provider/schema, and bodies
over 1 MiB. Use a bounded queue of 1,024 events; reject overflow with a diagnostic
counter rather than blocking agents or allocating indefinitely. Return 204 after
accepting an event into the queue. Acknowledgement does not promise durable
delivery. Never return provider decision JSON.

Forwarding network timeout is one second, with no synchronous retry. Normal
forwarding should complete within 100 ms at p95 on release test machines.
Events during shutdown or overload can be lost; reconcile from files and expose
uncertain state instead of promising complete event capture while the app is off.

### Configuration safety

The target integration screen has separate Claude and Codex enable/repair/remove
actions; the current helper's configuration-printing workflow is not this UI.
Installation is an explicit user action; startup never edits provider
configuration automatically. Detect non-default roots and permit an explicit
root override per provider. Do not scan arbitrary directories or credentials.

Use provider-compatible parsers: preserve unknown configuration fields and TOML
comments where applicable. Distinguish missing files from unreadable or malformed
files. Abort on invalid input; never substitute an empty configuration.

Track exact owned executable/handler registration, not substring matches.
Remove only owned handlers within mixed groups. Back up each changed file with
a collision-resistant name; write a temporary file beside it and atomically
replace after checking the original has not changed. If another writer changed
it, reread/remerge with at most one retry, then report a conflict without loss.
Do not overwrite backups or restore an old whole-file backup over newer settings
during uninstall. Leave disabled-by-policy settings untouched and explain why
live collection is unavailable. Verify installed executable and every required
registration rather than checking one SessionStart entry.

## 3. Shared contracts and behavior

The following fields describe the planned internal/IPC contract, not existing
public APIs. Use Rust as the source of truth and generate TypeScript bindings
when introducing the new model.

| Object | Required contract |
|---|---|
| SessionKey | provider + native session ID; include provider-native child ID for subagents |
| Session | key, optional parent key, host kind/name, cwd, repository/worktree IDs, branch, title, activity, status, attention, observation, source timestamps, optional process identity, metrics, available navigation actions |
| NormalizedEvent | event ID, session key, kind, received time, optional source time/sequence, optional turn/tool ID, sanitized metadata |
| Observation | source kind, last observed time, freshness: confirmed/stale/history_only; optional verified PID and process start time |
| IntegrationHealth | provider, configuration state, receiver state, last event time, supported capabilities, warnings/errors, detected version |
| Snapshot | monotonic revision, sessions for requested view, per-provider health, scan progress |
| NavigationResult | requested action, achieved action, success, explanatory message |

SessionKey must distinguish identical native IDs across providers. Resuming the
same native session updates its record. A genuinely forked session gets its own
record. If a provider exposes several configuration roots with colliding native
IDs, include the configured source-root identity in its key.

Keep host kind terminal/vs_code/desktop/headless/unknown separate from provider.
Store app display name independently. Do not label every process containing
“code” as VS Code or every “claude” parent as Desktop.

### State and evidence

Activity states: unknown, idle, working, blocked, errored, ended.
Attention reasons: permission, explicit_input, error, or none. “Idle” means the
last observed turn finished and the session is available for another prompt;
“Ended” means a session ended, not that its coding objective succeeded.

| Normalized event | Reducer behavior |
|---|---|
| Session observed/started/resumed | Create/update identity; idle only with evidence the session is waiting for a prompt |
| Prompt submitted | working; clear previous attention; start new turn identity |
| Tool started | working with tool description; preserve unrelated pending requests |
| Permission/input requested | blocked with reason and request identity |
| Request resolved/tool continued | Clear matching request; working when continuation is observed |
| Turn completed | idle; clear this turn's activity/attention |
| Turn failed | errored with sanitized explanation |
| Turn interrupted | idle with “Interrupted” activity if session remains available; otherwise unknown |
| Session ended | ended; clear attention and active work |
| Child lifecycle | Update child; never mark parent complete merely because a child stopped |

Not every tool failure is a failed turn. Unknown events can refresh observation
metadata but cannot arbitrarily change activity status. Represent concurrent
pending requests separately and retain blocked until all relevant requests
resolve or the turn ends. Aggregate children in details and counts without
duplicating them as unrelated top-level sessions.

Deduplicate by source event ID when available. Forwarders assign an ingestion
ID, which does not prove uniqueness of the underlying provider event. Where
source IDs/sequences are absent, avoid incrementing counters from hook arrivals;
use idempotent state updates and transcript reconciliation. Reject known older
turn/sequence updates; where order cannot be established, downgrade conflicting
status to unknown and reconcile. Never claim guaranteed ordering across providers.

Verified new transcript appends and supported hooks can establish observed
activity. Historical replay establishes metadata/counts only after adapter
validation; it cannot overwrite newer observation status. Missing approval
records in a transcript cannot clear hook-observed attention without evidence.
Null means unavailable, not zero. Show usage only with provider-specific
provenance; do not present tokens as comparable billing totals or USD.

### Freshness and recovery

Reconcile every five seconds while observing, and immediately after sleep/wake.
Use PID plus process start time to avoid PID reuse; process existence alone
does not prove an individual thread is working. A verified dedicated process
exit can end its session. A shared host staying alive cannot confirm all its
threads are active.

If no event or session-specific runtime evidence arrives for 60 seconds, mark
observation stale. Preserve the last observed activity as historical context,
but remove the “confirmed live” claim. Long tool runs may become stale; silence
must not change working to idle or ended. Retain stale pending attention with
a stale label and suppress repeat notifications.

On restart, load summaries immediately as stale/history_only and reconcile
before confirming current activity. A new event can restore confirmed status.
Missing SessionEnd, lost events, disabled hooks, and inaccessible files must be
visible limitations, not false success.

### Git identity

Run read-only Git queries with argument arrays, outside the reducer lock, with
a two-second timeout. Resolve checkout root and absolute Git common directory;
the canonical common directory identifies a local repository and the checkout
root identifies its worktree. Group linked worktrees together; separate clones
remain separate repositories even if they share a remote URL.

Resolve branch or show detached HEAD. Refresh after cwd changes, relevant Git
metadata changes, and manual rescan. Preserve native display paths while using
filesystem-aware identity; do not lowercase all paths on case-sensitive volumes.
Handle subdirectories, spaces, Unicode, linked worktree .git files, deleted
worktrees, non-Git folders, and unavailable Git without breaking collection.

### Storage and historical ingestion

Use SQLite in the platform app-data directory, with versioned migrations and
one background writer. Store normalized summaries, bounded sanitized activity,
source cursors, integration settings, and hidden-session preferences. Do not
modify provider databases or transcript files.

Discover provider roots at startup; add exact transcript paths received from
hooks rather than assuming all hosts use the same directory. Tail files by
identity/offset, buffer incomplete trailing lines, and handle truncation,
replacement, malformed records, and inaccessible files. Debounce file events;
use a 30-second reconciliation scan as a watcher fallback. Bound backfill
concurrency to two files and prioritize active/recent sessions.

Keep raw transcripts in their original location. Retain normalized activity for
seven days and historical summaries for 90 days by default; allow deletion of
Aperture history without deleting provider files. “Hide” persists across rescans;
“show hidden” can restore it. Keep a separate purge action for cached history
and explain that discovery can rebuild it from provider files. Back up the
database before schema upgrades; on migration failure, leave the original
intact and offer recovery rather than silently creating an empty replacement.

### IPC and UI updates

Add provider-scoped integration commands when automated setup is implemented;
the removed legacy installer is not an API to reuse. Extend rescan with
an optional provider and progress/partial errors. Session and navigation commands
take SessionKey, not a bare native ID. Add persistent hide_session (no active
forget command currently exists); use navigate_session(key, action)
for explicit navigation. These are internal breaking changes; no external API
compatibility promise exists for the current prototype.

Subscribe before requesting the initial snapshot and reject older revisions.
Handle subscription teardown even if unmount precedes registration completion.
Coalesce updates to at most ten snapshots per second; page history and avoid
emitting it all on every tool event. Update relative-time labels independently
of event arrivals.

## 4. Desktop experience and navigation

Default view: active/recent sessions grouped repository -> worktree -> session.
Show a unified attention area above groups, including both providers. Controls:
provider, status, repository, host, text search, and active/history/hidden views.
Activity details include a bounded timeline, source freshness, host evidence,
counts when known, children, and available navigation actions. Use folder name
plus short native ID as title fallback; never use a provider-generated title as
a unique identity. Empty states distinguish no sessions from disconnected
integrations and scans in progress. Keep port numbers in diagnostics.

Navigation order is capability-based: exact verified session target, known
app/window focus, open existing worktree folder, reveal known transcript.
Expose separate accurate action labels; do not silently report “opened session”
when only a folder was opened. If an action fails, offer the next fallback.
A request to navigate must never spawn/resume agent work or type into a terminal.

Use native Windows/macOS APIs for focus and file reveal, with allowlisted
provider URL schemes only when documented/tested. Validate destinations and
process identities at action time. A transcript path from an event is untrusted:
check it against configured roots or verified session provenance before enabling
reveal. Missing paths, denied OS automation permissions, or focus restrictions
produce an actionable result without repeated permission prompting.

Notifications are off until the user enables them. Once enabled, notify on
entry into permission/input/error attention, deduplicated per request/turn;
completion notifications are a separate off-by-default setting. Suppress replay
notifications during historical backfill/restart. Native notifications contain
provider/repository and a generic attention message by default, not prompt text.
Support keyboard navigation, visible focus, accessible status text, reduced
motion, OS scaling, and the existing minimum window dimensions.

## 5. Implementation phases

Phases are ordered by dependencies, not calendar promises. Phase 0 has a
validated Windows passive milestone and an active Windows follow-up; Mac runtime
validation remains externally pending. Later Windows engineering may proceed
without representing Phase 0 or the cross-platform release as complete.

### Phase 0 — Right now: two-provider compatibility proof

Deliver: passive two-provider baseline, provider-qualified identity, normalized
boundary, optional safe hook enrichment for supported missing signals,
provider badges/health, sanitized real fixtures, and the 12-row matrix in SPIKE.md.
The legacy installer has been removed; require safety before any automated setup. Validate event mapping,
discovery, process attribution, and navigation capabilities before relying on
them in later phases.

Exit: both externally started providers update together; real core lifecycle
evidence exists across required hosts/OSes; malformed settings and mixed handlers
are preserved; closed-dashboard behavior is observational. Minimum supported
provider versions and tested OS/architecture combinations are recorded. Missing
required observation remains a blocker; exact-session navigation can explicitly
fall back. This phase does not claim durable history or production reliability.

### Phase 1 — Reliable collection and recovery

Deliver: bounded trusted local ingestion, packaged observational helper, idempotent reducer,
child/request handling, SQLite/migrations, incremental history readers for both
providers, freshness reconciliation, integration repair, and collector/scan health.
Complete the IPC revision/subscription changes.

Exit: restart, interrupted file writes, duplicate/delayed events, disabled hooks,
inbox bounds/corruption, process exit/reuse, and sleep/wake tests pass. If HTTP is
introduced, also verify authentication and port collision. No stale session
is reported as confirmed working without evidence. Counts survive reconciliation
without hook/transcript double counting. Provider configuration remains intact
through repeated install/repair/remove and concurrent edits.

### Phase 2 — Repository/worktree dashboard and navigation

Deliver: Git identity service; mixed-provider repository/worktree grouping;
attention, filters, details, child display, persistent hide; Windows/macOS
navigation with structured outcomes and accurately labeled fallbacks.

Exit: two repositories with multiple linked worktrees and both providers group
correctly, including multiple sessions in the same checkout. Every required host
has a tested navigation action or folder/transcript fallback. Wrong/reused PID,
deleted worktree, denied focus, and unsupported deep links cannot open the wrong
session while claiming success. R1-R10 and R12 have end-to-end coverage.

### Phase 3 — Daily-use desktop beta

Deliver: tray/menu-bar lifecycle, single-instance handling, opt-in notifications,
accessible/responsive UI, bounded retention, diagnostic export/recovery controls,
performance instrumentation, and packaged internal Windows/macOS builds.
Bundle fonts locally so ordinary dashboard rendering does not need network access.

Exit: representative mixed-provider daily workflow passes an eight-hour soak
with sleep/wake and app/provider restarts. Notification deduplication, keyboard
use, scaling, retention, and privacy checks pass. Performance targets below pass.
Publish an explicit supported-version matrix and known limitations; no host is
called supported based solely on documentation. R11 has end-to-end coverage.

### Phase 4 — Production ready state

Deliver: signed Windows installer and signed/notarized macOS distribution;
release CI for advertised architectures; tested upgrade/database migration and
rollback/recovery; clean install/uninstall; versioned release notes, user guide,
compatibility/troubleshooting documentation, and a support/diagnostic workflow.
Use manual signed release upgrades initially; automatic updates are not required.

Exit: all requirements R1-R12 and quality gates pass on advertised systems.
No known release-blocking configuration loss, false session identity, ingestion
security failure, or required-host observation gap remains. A 24-hour mixed
provider soak shows no sustained memory growth, stuck confirmed status, or
unbounded queues. Install/upgrade/uninstall is tested on clean user profiles.
Removal can clean Aperture-owned hooks while preserving all agent history and
unrelated settings. Publish artifacts only after these gates are recorded.

## 6. Verification and release evidence

| Test family | Required cases and evidence |
|---|---|
| Adapter fixtures | Real sanitized Claude/Codex payloads; unknown fields/events; errors; permission/input; interruption; child lifecycle; recorded provider versions |
| Identity/reducer | Same native ID across providers; resume/fork; two sessions in one cwd; stale/duplicate/out-of-order events; child completion; multiple pending requests |
| Installer | Missing/malformed/unreadable JSON/TOML; mixed handlers; similarly named unrelated commands; repeat install; concurrent edits; backup collision; explicit removal |
| Ingestion | Invalid schema/body size; local inbox bounds/ownership/corruption; app closed; one provider unavailable while the other works; no decision output; token/port/HTTP queue checks only if HTTP transport is introduced |
| Historical recovery | Partial JSONL, rotation/truncation, missing files, different roots, duplicate usage records, interrupted migration, restart and hidden-session persistence |
| Git/navigation | Linked worktrees, subdirectories, separate clones, detached HEAD, no Git, Unicode/space paths, deleted paths, stale PID, OS focus denial, truthful fallback labels |
| UI/lifecycle | Snapshot race/cleanup, attention ordering, filters, empty/partial states, notifications, hide/restore, tray/quit, keyboard/scaling/reduced motion |
| Real compatibility | All 12 provider/host/OS combinations; externally started sessions before and after Aperture; repeat after provider upgrades |
| Distribution | Clean installs, signed artifacts, upgrade/migration recovery, uninstall hook cleanup, explicit retained app-data behavior |

Run npm run typecheck, npm run build, and cargo test --manifest-path
src-tauri/Cargo.toml on implementation changes, plus relevant integration suites
introduced by the phases. These commands alone do not prove host compatibility.

Performance fixture: 50 active sessions, 10,000 historical summaries, five
repositories, and multiple worktrees; steady 20 events/second with 100/second
bursts for ten seconds. Measure p95 ingestion-to-visible latency <=1 second,
persisted initial results <=2 seconds, normal forwarding <=100 ms, and bounded
queue/memory behavior. Record hardware, OS, versions, sample count, and results.
Backfill progress must remain usable under this load. During idle soak, target
average process CPU below 2% and working-set memory below 300 MiB on the recorded
release machines. If targets fail, profile and fix or explicitly revise the
requirements before asserting production readiness.

Maintain evidence records per phase: commit/build ID, environment/version,
procedure or test command, result, fixture/log reference, limitations, and owner.
Mark tests pending when no suitable machine exists; never infer macOS results
from Windows. External sessions used for manual validation must be disposable
test tasks, not interrupted real user work.

## 7. Integration evidence, assumptions, and open empirical questions

Official sources inspected during the requirements assessment:

- [Claude hooks](https://code.claude.com/docs/en/hooks): lifecycle event inputs
  and HTTP hooks. This supports the observer approach, not a universal transcript
  or window-identification contract.
- [Claude desktop](https://code.claude.com/docs/en/desktop): shared local settings.
- [Claude VS Code](https://code.claude.com/docs/en/vs-code): extension settings.
- [Codex hooks](https://learn.chatgpt.com/docs/hooks): lifecycle events and
  transcript-path input; transcript format is explicitly not stable.
- [Codex app server](https://learn.chatgpt.com/docs/app-server): task listing and
  status events for connected server tasks. Machine-wide attachment to arbitrary
  independently running instances is not established.

Do not treat tool access inside this Codex conversation as an API automatically
available to a standalone Aperture installation.

Confirmed decisions: both providers; external ownership; monitor-and-jump; local
Windows/macOS; multiple repositories/worktrees; no remote aggregation or control.

Engineering defaults chosen here: retain Tauri; packaged hook forwarder; SQLite;
unknown/stale evidence instead of guessed status; separate provider collectors;
local-only data; optional notifications; explicit navigation fallbacks; manual
signed upgrades. They are design choices, not claims about existing behavior.

Phase 0 must resolve exact supported versions, effective root/schema differences,
hook coverage per host, process ancestry, and navigation targets. Pin the results
in the compatibility matrix before claiming support. A failed assumption leads
to a documented adapter revision or a clearly reported release blocker. It must
not turn the product into a launcher or quietly exclude Codex, Claude, or an OS.
