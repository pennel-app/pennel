# Recording-shortcut locale correction — scoped review

Reviewed only the current four-file parent diff in `agent-a0097254ec71636e7`: `record_shortcut.rs` and backend EN/NL/source catalogs. No broad audit, source/index/HEAD changes, test/build reruns, CI, native checks, publishing or subagents.

**Specification verdict: PASS. Quality verdict: PASS. The language-policy root cause is addressed in source; no new actionable regression found in this correction.**

`record_failure_message()` retains the existing captured shortcut label and Dutch/English presentation decision. It selects one of two exact catalog keys based on the same `label.is_empty()` condition, then calls the established Core `ui_text` path (`record_shortcut.rs:66–81`). There is no new settings read, provider work, callback/lock behavior or change to shortcut identity/capability/dispatch.

The EN/NL catalog values preserve all four previous outputs exactly. The named key has the same single `{label}` placeholder in English, Dutch and sources; the unnamed key has none. The supplied extra argument on the unnamed path is safely ignored. Core's renderer scans placeholders in the template and appends argument bytes directly, so a label containing braces or text resembling a translation cannot be recursively expanded. The existing source alias machinery also keeps these owned messages translatable when they pass through the usual UI localization boundary. No language-policy allowlist or test exception was added.

Evidence read without rerunning:

- The concrete original red was inspected in the failed CI logs: exactly the two Dutch runtime literals at the old `record_shortcut.rs:73/76`. The parent additionally reports reading the same local two-test red before editing. I inspected the durable `/tmp/pennel-final-locale-policy.log` green: **2 tests / OK**, no policy exception.
- `/tmp/pennel-final-locale-core.log`: **11 localization tests passed**, including catalog/source key and placeholder parity, exact owned matching and nonrecursive literal substitution.
- `/tmp/pennel-final-locale-frontend-build.log`: TypeScript/Vite build passed; only the existing large-chunk advisory remains.
- `/tmp/pennel-final-locale-frontend-tests.log`: **524 passed, 0 failed**.
- `/tmp/pennel-final-locale-backend.log`: **407 passed, 0 failed**, followed by successful strict backend clippy completion (`Finished dev profile`); this log was read after completion.

A new parent-owned durable combined selector run in `/tmp/pennel-final-selectors.log` hit a separate local Python compatibility error before the chained policy step: `tarfile.extractall(..., filter='data')` rejects `filter` in this interpreter (`test_ci_xcode_cache.py:165`, `ci-xcode-cache.py:251`). The affected remote selector jobs previously passed all 162 selector tests. This local interpreter mismatch is outside the four-file correction and is not attributed to these messages. I subsequently read `/tmp/pennel-final-selectors-supported-python.log`: the same selector suite passes **162 tests / OK** with the existing supported Python 3.11. The standalone two-test policy green is separately preserved as above; no test or policy change was needed. The parent owns the final canonical verification record.

No remote check has been inferred green from local evidence. Publishing/backports, PR status refresh and the separate PR #239 Mac packaging retry remain parent-owned.

**Requested correction: none.**
