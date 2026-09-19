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

_No entries yet._
