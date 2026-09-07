# Aperture detailed specification and implementation phases

Status: proposed implementation contract for the agreed product requirements.
No implementation phase is marked complete by this document.

[Requirements](requirements.md) define the product scope.
[Phase 0](../SPIKE.md) holds the immediate checklist and compatibility evidence.
If an integration experiment contradicts this design, record the evidence and
amend the adapter design; do not quietly narrow the requirements.

## 1. Current state and design delta

The current application uses Tauri 2, Rust, Tokio/Axum, React 18, and TypeScript.
Retain this stack. The observer core is already separate from the Tauri bridge,
which makes provider adapters possible without replacing the desktop shell.

| Area | Current evidence from source | Required change |
|---|---|---|
| Ingestion | Claude-specific POST /hook and shell/cmd wrappers | Provider-specific parsing behind a shared event contract |
| Identity | Store indexed by raw session_id | Provider-qualified identity; optional parent identity |
| Configuration | One hooks_installed boolean; marker substring matching | Independent integration health and safe per-provider installation |
| Status | live stays true after any hook; no reconciliation | Separate activity state, observation freshness, and process evidence |
| History | Whole-file startup/manual Claude scan; memory-only state | Provider discovery, incremental reading, durable summaries and cursors |
| Git | repo_root exists but is never populated | Repository/worktree identity derived from Git |
| Navigation | Experimental AppleScript; Windows returns text | Verified per-host actions with structured results |
| UI | Cards, permission strip, full snapshots | Mixed-provider grouping, broader attention, filters, details, health |
| Verification | Five Rust tests; TypeScript/build passed in prior assessment | Real compatibility fixtures, OS smoke tests, failure and release checks |

Known correctness issues to address: malformed settings can be replaced by an
empty object; matching one Aperture handler can remove a whole mixed hook group;
the health label can claim listening after bind failure; hook counts and
transcript counts disagree; current error-detail parsing does not match the
documented Claude string error; child-agent events can affect parent state;
frontend initial fetch and subscription can race and asynchronous cleanup can
leak listeners. Historical scans do not prove live status. These are code
findings, not reports of a completed end-to-end test.

## 2. Architecture and ownership

Flow: provider hook -> local collector -> provider adapter -> normalized event
queue -> session reducer and storage -> Tauri snapshot -> dashboard.

Historical flow: provider discovery -> incremental transcript parser -> metadata
and usage reconciliation -> same storage and dashboard.

Supporting services: read-only Git resolver, process/host resolver, native
navigation, integration configuration, and diagnostics.

Use one local observer owned by the desktop process. Closing the window hides
it to the tray/menu bar after onboarding explains this behavior; explicit Quit
stops observation. Do not introduce a privileged OS service. Enforce one app
instance so two windows cannot compete for the listener or database.

### Provider adapters

Implement Claude Code and Codex as separate modules. Each owns:

- Effective configuration-root discovery and supported-version diagnostics.
- Hook configuration install/check/remove and external payload parsing.
- Historical discovery, transcript schema versions, and usage interpretation.
- Event-to-state translation and any verified provider-specific navigation.

The shared reducer must never switch on raw provider hook names. Use explicit
provider values claude_code and codex, independent of model names or API vendors.
Keep extension points small; a general plugin SDK is outside this release.

For Phase 0, retain command-hook forwarding and add a small packaged Rust
forwarder executable to replace brittle JSON splicing and shell assumptions.
It reads stdin JSON, attaches provider/source identity and bounded process
metadata, posts to loopback, emits no stdout, and exits successfully even when
delivery fails. Use an absolute executable path with provider-supported argument
encoding. Validate paths containing spaces, Unicode, and shell metacharacters.
Do not assume the immediate parent PID is the agent; resolve ancestry and record
unknown when identity cannot be proved. Never collect the full environment or
raw process command lines for telemetry.

Direct HTTP hooks can later replace forwarding where both payload coverage and
host attribution are adequate. They are not a prerequisite. Codex app-server
observation is an optional adapter experiment, not a machine-wide discovery
assumption and not a reason to resume an external session.

### Local ingestion

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

The integration screen has separate Claude and Codex enable/repair/remove
actions. Installation is an explicit user action; startup never edits provider
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

Hooks establish observed activity. Transcripts establish historical metadata and
counts only after adapter validation; a file scan cannot overwrite newer live
status. Null means unavailable, not zero. Show usage only with provider-specific
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

Replace unqualified install/uninstall commands with provider-scoped integration
commands. Rescan takes an optional provider and reports progress/partial errors.
Session and navigation commands take SessionKey, not a bare native ID. Replace
forget_session with persistent hide_session; use navigate_session(key, action)
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

Phases are ordered by dependencies, not promised calendar dates. Documentation
completion does not imply Phase 0 implementation has started.

### Phase 0 — Right now: two-provider compatibility proof

Deliver: installer safety prerequisite, provider-qualified identity, normalized
boundary, Codex hook collector, provider badges/health, sanitized real fixtures,
and the 12-row compatibility matrix in SPIKE.md. Validate event mapping,
discovery, process attribution, and navigation capabilities before relying on
them in later phases.

Exit: both externally started providers update together; real core lifecycle
evidence exists across required hosts/OSes; malformed settings and mixed handlers
are preserved; closed-dashboard behavior is observational. Minimum supported
provider versions and tested OS/architecture combinations are recorded. Missing
required observation remains a blocker; exact-session navigation can explicitly
fall back. This phase does not claim durable history or production reliability.

### Phase 1 — Reliable collection and recovery

Deliver: authenticated bounded ingestion, packaged forwarder, idempotent reducer,
child/request handling, SQLite/migrations, incremental history readers for both
providers, freshness reconciliation, integration repair, and listener/scan health.
Complete the IPC revision/subscription changes.

Exit: restart, interrupted file writes, duplicate/delayed events, disabled hooks,
port collision, process exit/reuse, and sleep/wake tests pass. No stale session
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
| Ingestion | Invalid token/schema/body size; queue overflow; port occupied; app closed; one provider unavailable while the other works; no decision output |
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
