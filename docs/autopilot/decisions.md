# Decisions made without you — aperture

Every entry here is a design question the requirements and specification did not
settle, which a build routine resolved on its own so the slice could ship.
Appended by the routines; newest last.

Per the design's ambiguity policy, a routine picks the most defensible option,
records it here and in the PR body, and proceeds. It does not stop, and it does
not guess silently.

A routine may **not** use this file to record anything about host compatibility,
runtime behavior, or validation status. Those belong to you and to
`docs/validation/`, which autopilot cannot write.

**Read this before reviewing the `auto/queue` diff.**

Format:

```
## <date> — slice-<n>, PR #<n>
Question:    <what the spec left open>
Chosen:      <what was done>
Rejected:    <the alternative, and why not>
Blast radius: <files or functions affected, and how reversible>
```

---

## 2026-09-21 — slice-1, PR #33
Question:    Issue #18 asks for a ruling it deliberately leaves open: does
             `Session` get `#[serde(default)]` / tolerant deserialization so
             additive shape changes stop orphaning persisted rows, or are
             breaking shape changes accepted as a now-visible migration cost?
Chosen:      Accept breaking shape changes as a now-visible migration cost, and
             add **no** `#[serde(default)]`, because measurement showed the
             tolerant half was already there. The first draft of this slice did
             add `#[serde(default)]` to the eight `Option` fields; running the
             new tests against a reverted tree to check they actually fail
             without the change showed
             `a_row_written_before_an_optional_field_existed_still_loads`
             passing either way. Serde's derive already resolves a missing
             field for `Option<T>` to `None`, so those eight attributes were
             no-ops. They were removed rather than shipped as decoration that
             implies a mechanism it does not provide. The test stayed: it now
             pins serde's behavior so a later change cannot quietly remove it.

             So the policy, written down rather than newly built: an additive
             `Option` field (`SessionKey` in #9, Git identity in #7) already
             costs no stored history, and `None` is an honest "not known". The
             required fields (`id`, `provider`, `native_id`, `attention`,
             `observation`, `status`, `host`, `cwd`, the counters, `live`) stay
             strict, so a breaking change to those is a migration cost
             `load_summaries` now surfaces instead of swallowing. The other
             direction — an older build reading a row a newer build wrote —
             was already tolerant, since serde ignores unknown fields.
Rejected:    (a) Blanket tolerance via container-level `#[serde(default)]`.
             It needs `impl Default for Session`, and it would turn a corrupt
             row into a session with an empty `id`, an empty `attention` and a
             manufactured `status` — a plausible-looking session assembled out
             of unreadable data. That is the same "silent loss presented as a
             clean state" this issue exists to stop, wearing a different hat,
             and it would defeat the detection the rest of the slice adds.
             (b) Keeping the eight no-op `#[serde(default)]` attributes as
             documentation of intent. A reader would reasonably assume they
             were load-bearing, and a future field added as non-`Option` would
             inherit that false assurance. The doc comment on `Session` says
             the same thing without pretending to do work.
Blast radius: `src-tauri/src/observer/model.rs` — comment only; no attribute,
             field, rename or removal, so `Session`'s serialized form and the
             frontend contract in `src/features/sessions/types.ts` are
             untouched. The behavior this entry rules on is serde's, not this
             repo's, so there is nothing here to revert.

## 2026-09-21 — slice-1, PR #33
Question:    Where should a failed row surface? The issue asks for "log output
             at minimum" and `docs/roadmap.md` Phase A item 1 says "surface
             load failures in the storage health entry", but summaries are
             loaded exactly once at startup while that health entry is rebuilt
             on every reconcile cycle, so a per-load return value alone cannot
             reach it.
Chosen:      Both, from one source. `load_summaries` returns `SummaryLoad
             { sessions, failed }` and logs each failure to stderr (capped at
             10 lines plus a total, so a broadly corrupt database cannot bury
             every other startup message), and `Db` keeps the failure count
             from the most recent load so `commands::storage_health` can report
             it every cycle. The health row goes `degraded` with a detail that
             says the rows were kept, not deleted. The count is per-load, so it
             clears once a load succeeds cleanly.
Rejected:    (a) Log only. Satisfies the issue's minimum but not the roadmap's
             "surface in the storage health entry", and it is discoverable only
             by someone watching stderr. (b) Recomputing the count inside
             `storage_health` by re-reading every row each cycle: it would turn
             a startup-only cost into a per-cycle full table scan, against the
             write-amplification work in the 2026-09-10 persistence design.
Blast radius: `src-tauri/src/observer/db.rs`, `src-tauri/src/commands.rs`
             (`storage_health`), `src-tauri/src/lib.rs` (one call site).
             `storage_health`'s existing `"ok"` / in-memory / write-failure
             details are unchanged; the new clause is appended, so a cycle that
             is both write-degraded and missing rows reports both. No schema or
             migration change — `SELECT id, data` reads a column migration 1
             already created.

## 2026-09-23 — slice-2, PR #34
Question:    Issue #17 lists three candidate eviction policies (age-based, a
             bounded LRU-style cap, or hiding from the default view) and does
             not choose. `docs/roadmap.md` Phase A item 1 proposes 14 days as
             a *proposal*, and `docs/specification.md` §"Storage and historical
             ingestion" names two other numbers nearby: seven days for
             normalized activity, 90 days for historical summaries. Which
             number governs the live store, and which shape of policy?
Chosen:      Age-based, 14 days, from the roadmap's proposal. Nothing in the
             code made it untenable, which the queue set as the bar for
             departing from it.
             Age beats an LRU cap because a cap has to be tuned against a
             number nobody has measured yet — issue #13 has not run the scale
             fixture — and because the failure mode of a wrong cap is worse:
             it evicts by rank, so on a busy machine a session the owner is
             actively watching can be pushed out by newer ones. Age evicts
             only what nothing has observed for two weeks, so the dashboard's
             contents depend on this machine's activity and not on how many
             other sessions happen to exist.
             14, not 7: seven days is the retention window for normalized
             activity records, a different thing from a session summary, and
             borrowing it would evict sessions from a fortnight-long piece of
             work that the owner would reasonably still expect to see. 14, not
             90: matching retention would make the live store hold the entire
             persisted history, which is the growth this issue is about.
             The threshold is `state::LIVE_STORE_IDLE_DAYS`, one constant, and
             both call sites take it as an argument so the policy is visible
             where it is applied and testable without waiting two weeks.
Rejected:    (a) A bounded LRU-style cap — see above; revisit if #13's scale
             numbers show age alone leaves the store too large on a busy
             machine, since the two compose.
             (b) Issue #17's third option, "hide from the default view while
             keeping them queryable", read literally: keeping every session
             resident and filtering at render time. It fixes the dashboard and
             not the working set, and the working set is what the issue,
             `docs/requirements.md` §4 and the 24-hour soak gate are about.
Blast radius: `src-tauri/src/observer/state.rs` (new `evict_idle`, a filter in
             `restore_summaries`, one counter field), `src-tauri/src/
             commands.rs` (`reconcile_and_persist`, `storage_health`),
             `src-tauri/src/observer/db.rs` (`forget_cached_summaries`),
             `src-tauri/src/lib.rs` and `src-tauri/tests/durable_recovery.rs`
             (call sites). No schema change, no migration, no frontend change.
             Fully reversible: set `LIVE_STORE_IDLE_DAYS` arbitrarily high and
             the behavior is the old one, since nothing is ever deleted.

## 2026-09-23 — slice-2, PR #34
Question:    Where in the reconcile cycle does eviction run? The issue says
             "wire `Store::remove` into the reconcile path" without saying
             where, and the two obvious positions are not equivalent.
Chosen:      After `save_summaries`, not before. Backfill means a session can
             be discovered already past the threshold — `Observer::poll`
             reading a months-old transcript for the first time inserts it
             with that old `last_event_at` — so evicting before the save would
             drop it before its summary row was ever written. "Evicted
             sessions remain visible in history, backed by the durable SQLite
             summaries" would then be false for exactly the sessions most
             likely to be evicted. Saving first makes the durable row the
             thing eviction falls back on, which is what the criterion asks
             for. `push_snapshot` runs after `reconcile_and_persist` in both
             call sites, so the window still never renders the evicted
             session.
             Same cycle, the ids `evict_idle` returns are passed to
             `Db::forget_cached_summaries`, because `Db`'s write-skipping
             `summary_cache` holds a serialized copy of every summary written
             this process. Bounding `Store.sessions` while leaving that map
             unbounded would move the growth rather than remove it, and the
             cached JSON is larger than the `Session` it stands for.
Rejected:    (a) Evicting before the save — see above. (b) Leaving the
             `summary_cache` alone and saying so in the PR body. It is four
             lines to do properly, and an eviction policy whose memory saving
             is cancelled by another map is not worth reviewing.
Blast radius: `src-tauri/src/commands.rs` (`reconcile_and_persist`, ordering
             only) and `src-tauri/src/observer/db.rs` (one new method that
             touches no SQL). Forgetting a cache entry is safe by
             construction: a cache miss makes the next save write the row
             rather than skip it, so the worst case is one redundant write
             when an evicted session is resumed.
