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

## 2026-09-21 — slice-1, issue #18
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

## 2026-09-21 — slice-1, issue #18
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
