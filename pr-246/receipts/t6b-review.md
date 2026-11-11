# T6b suggestion consent/UI/caller independent review

Reviewed final pin `6ca385426413c2cda08122dc90baa013718eac8e` on frozen `a463cdabec735510c0f60e9d66d79a9e91449d42`, using `t6b-complete-review.diff` for the two feature deltas (`af2027731a621b9282b091aa2975c9b6f58f7d08` and final `6ca3854`). The already reviewed helper trio and duplicate-only `1dc1ae7` were excluded from the milestone delta. Read `t6b-report.md`, accepted suggestions ADR and constraints; inspected pinned source and supplied receipts. No source/index/HEAD changes, test/build reruns, subagents, native checks, CI or external writes.

**Spec verdict: pass. Quality verdict: pass. No proven actionable correctness, security or API finding in this milestone.**

## Reviewed contracts

Suggestion consent remains independent of Summary. The unchecked/checked Find mapping saves Manual/Automatic respectively and waits for acknowledgement before using the existing manual analysis path. An explicit unresolved language becomes the saved fallback only with checked Automatic consent. Quiet Find starts once without a policy write. Not now writes its own durable meeting metadata flag and leaves both global policies unchanged.

The dismissal reuses guarded core meeting/source/revision CAS, preserves the other flag and canonical content/record, and is acknowledged through Summary's retained metadata authority. The desktop writer retains `.then(validated)`, invalidates reads before the write and its acknowledgement, and uses the combined state that clears deferred reader presentation while retaining typing and candidate revision ownership. The consent component invalidates its own reads before choices and after acknowledgements; late values/errors cannot replace a durable quiet state. Read failure Retry, external durable dismissal refresh and newly saved fallback adoption remain reachable without inference.

The optional language preview is setup-only. It reuses recorded-language resolution, exact manual qualification and the additional foreground-impact requirement; supported recorded language wins and an unsupported explicit recorded language is not relabelled by fallback. Rendering performs no policy/marker write, credential lookup or provider call. Settings displays saved local connection/policy/fallback data and conditional qualification wording without invoking the generic network-capable readiness probe.

The presentation retains cards/edits/evidence/source and insertion routes, accepted export, read-only store warnings, consumed/stale/damaged/interrupted/failed attempt explanations and reachable Connection settings repair. It does not retry automatically. Existing retained row/control ownership remains in place through status updates. English/Dutch strings and responsive controls use the shared UI patterns and selected-state token.

The runtime has one caller immediately after the summary block, including summary error/skip paths. The metadata-only whole-postprocess span starts before speaker preparation and ends after the reviewed suggestion helper or an early return. Its independent stage identity does not replace the summary/suggestion outcome contracts. Preparation disables competing Find, while Cancel is exposed only for a real owned analysis job. Admission, credential ordering, draining and final durable receipt barriers remain with the already reviewed helper/core.

The workspace delta adds the three suggestion metadata callbacks and readiness forwarding. It changes no Notes/source/insertion facade contract or editor internals. Combining the separately reviewed T4 implementation remains parent-owned integration work.

## Evidence and acceptance limits

Read final logs confirming **1292 core passed / 7 ignored**, **392 backend passed**, **502 frontend passed**, frontend build and strict core/backend clippy. Accepted supplied fmt/theme/diff receipts without rerunning them. The known nonfatal chunk-size warning is disclosed by the report and is not an additional finding.

Read complete manifests with no recorded runtime/surface errors, horizontal overflow or unknown commands: **16** EN/NL/light/dark wide/minimum/enlarged frames, **13** preservation, **7** async ownership, **3** repair, **5** actions and **8** state cases. Read the six paired reference inputs (four Review and two Settings) and inspected enlarged NL/dark Review and EN/light Settings captures. The focused production-owner cases exercise acknowledgement-before-late-value/error, independent durable Not now, no-work rendering, explicit fallback, preparation gating and repair; these are accepted recorded results, not reviewer reruns.

The adopted qualification registry is still empty, so production analysis remains unavailable until exact evidence is adopted. Synthetic qualification demonstrates UI/lifecycle contracts, not provider/model quality. Real credentials/provider execution, physical capture, native accessibility/VoiceOver, signed/device/mobile runtime, Windows/Linux and release acceptance remain open as documented. No acceptance claim is inferred from this source review beyond the scoped milestone.

**Requested corrections: none.**
