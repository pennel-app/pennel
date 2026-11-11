# Finished Review qualification — 11 November 2026

Owner-approved work under [#96](https://github.com/pennel-app/pennel/issues/96), based on released main `27b75bec`. Both selected routes received the unchanged four-question production request for an invented Dutch development meeting. **Both stopped at `identity_unavailable`; zero records were adopted.** Review's production gate and settings remain unchanged.

## Actual model observations

| | Retained local Decider | Pinned hosted Decider |
| --- | --- | --- |
| Model | `pplx-decider-flash-v1.2` | `perplexity/pplx-decider-v1.2` through OpenRouter, upstream `perplexity` |
| Actual returned model | `pplx-decider-flash-v1.2` | `perplexity/pplx-decider-v1.2-20261016` |
| HTTP / executed inference requests | 200 / 1 | 200 / 1 |
| Provider / revision | Both omitted | `Perplexity` / omitted |
| Required identity outcome | Unavailable | Unavailable |
| Request elapsed | 1,633.17 ms | 718.63 ms |
| Returned billed cost | No hosted call | US$0.00001416 |

Each response contained all four answer IDs: action, decision, open question and changed commitment. Their raw bodies omit the required identity fields; the parser is not dropping supplied metadata. No alias, file name, public checkpoint manifest or proxy was used to manufacture missing serving identity. Successful compatibility does not establish quality or qualification.

The local trial reused the personally retained official Decider 0.1.0 binary and existing default GGUF. The model's SHA-256 remained `ffb27ade8976765b0e6943a0b7f5f3fd0f7042d45ad90256a76d608bcd3942c2`; the owned running process had that model file open. This supports local artifact/loading provenance, but it is not a production per-response serving-attestation contract. The signature and binary hash were checked before launch. Explicit `--model` and `--no-vision`, together with offline model/cache settings, avoided acquisition. **New model/runtime downloads: zero bytes.**

Only the trial's process group was started and stopped. Official model-ready logging and an owned listener established readiness after 2.28 seconds; evaluation stopped 1.76 seconds after readiness, within the two-hour cap. The owned runtime's entire startup/request/shutdown lifetime measured 5,912,625,152 bytes peak RSS and 1,020 ms user+system CPU. Those are scoped observations from one request, not cold/warm qualification samples or aggregate Pennel acceptance. The owned runtime was confirmed stopped.

The hosted trial verified the current exact upstream's catalog and public ZDR eligibility, sent the production privacy/routing constraints unchanged, and reserved US$0.10 before its sole request under the new US$2 total ceiling. The returned US$0.00001416 charge settled the reservation; pending reservations are empty and no charge is unresolved. No retries, fallback route or personal meeting content were used. These facts do not assert regional residency, account logging settings or independently verified remote model weights.

## Implemented evaluation path

The historical `finished.py` pilot/protocol and incremental results remain unchanged. The new `finished_qualification.py` export/replay path binds the real Rust production export, raw parser and candidate decoder to frozen Dutch development/calibration/held-out families, authored labels and complete source-bound item judgments.

- Calibration uses only its split and exact prompt/ordered-label/grouping/model/provider/revision/artifact context. A fitted artifact is frozen before held-out request export; altered source, context, inputs or artifact are rejected.
- Raw production classification, calibrated classification and complete-item quality are separate. Every raw proposed item is checked for exact kind, commitment, quote, source bytes/fingerprint, supported owner and verbatim due phrase. Calibration cannot hide raw unsupported proposals. Missing/failed windows remain in denominators, and incomplete production jobs receive no committed-item credit.
- Component and complete-item macro-F1 retain the 0.90 gate; accepted-item precision retains 0.95. Calibration retains at least ten observations, two classes and two meeting families per exact context.
- Every original calibration and held-out job requires positive `measurements.whole_job_elapsed_ms`, measured from production preparation through final raw replay, within the frozen whole-job deadline. A request-only timing or separate timing panel cannot substitute. Imported successful jobs beyond that deadline are refused.
- A source-bound fixed five-cold-client/thirty-warm complete-job panel is scored separately and adds no quality credit. It requires executed-request counts and appropriately scoped actual CPU/RSS; local warm p95 retains the five-second gate. Hosted server CPU/RSS remain explicitly unavailable.
- Perfect offline fixture predictions cannot qualify. Incomplete original-job timing, missing resources, uncalibrated contexts or non-recorded inference block candidate records. Even a fully passing candidate is only a review artifact: this runner never writes the registry or changes production admission. Foreground/background, natural-ASR and human acceptance remain separate.

`finished_preflight.py` executes only the two explicitly selected development routes. It sends exported production bytes, retains raw consumed attempts on provider/transport/bridge failure, uses a whole-job deadline and at most two hours overall including preparation, and stops on the first identity or budget failure. Hosted work uses the existing persistent reservation ledger; unknown/malformed charges remain reserved and stop further paid work.

## Commands and retained evidence

The fresh lean evaluation binary was built without downloads:

```sh
scripts/with-toolchain.sh cargo build --offline --locked --manifest-path core/Cargo.toml --no-default-features --features evaluation --bin pennel-eval
python3 -B -m unittest discover -s evaluations/decision-models/tests -p 'test_finished*.py'
python3 -B -m unittest discover -s evaluations/decision-models/tests -v
```

Verification passed: **317 full Python tests**, followed by **30 final focused finished-pipeline regressions** after the last error/deadline hardening. The final shared-core source also rebuilt the lean evaluation binary successfully. No production Rust code or application qualification registry was changed by this evaluation work.

Local runtime orchestration ran through `worktree-storage run`; the hosted request used only the existing evaluation Keychain credential in process memory. Credentials were not written to evidence or printed. Exact scoped commands, synthetic request/response bodies, both preflight freezes, current catalog/ZDR snapshots, runtime logs, open-file observation, settled ledger and machine receipt are retained under ignored `build/review-qualification-96/20261111/`. The copied evaluation binary is pinned to SHA-256 `589ad6cd42ae0910b4fedf52511a229fbb4e3c6d0ddaea0fd58fb49159271f8c`.

The final hardened runner was checked by replay/tests after the actual preflights; error/deadline retention repairs do not change those successful wire bytes or raw replies. Neither actual model request was repeated.

After the independent automatic-title work settled, a separate final-source binary was built and retained as `final-pennel-eval`, SHA-256 `542c9cbd273e919ded70db079d467cba02a12fe9a0c7eb2178ff0ec45386681d`. Both routes were re-exported and their original raw responses replayed **offline**. Every inner export field (fixtures, configuration, expected identity, production qualification request, source hashes, complete window/wire/source mappings, snapshot and inner freeze hash) matched the historical trial export exactly. Both final replays again returned `identity_unavailable`. `final-source-parity.json` and the final freezes/replays retain that comparison. Original measured timings remain attached to the original trial binary, not relabelled as a new runtime measurement. New model requests during final-source verification: **zero**.

Full replay usage is explicit: `finished_qualification.py freeze --input SPEC.json --bridge BINARY --output FREEZE.json`, followed by `requests --split calibration`, `calibrate --predictions CALIBRATION.json`, `requests --split heldout`, and `evaluate --predictions HELDOUT.json --resources RESOURCES.json`, using the frozen output as `--input` each time. Existing output files are preserved. This is an export/replay qualifier; model execution is separately bounded and authorized.

## Remaining blocker

Both selected routes lack required serving identity. Calibration, held-out quality, complete-item judgments of actual output, the fixed resource panel and foreground-impact acceptance were therefore **not attempted**. No fabricated successful score or qualification record substitutes for them. Keep #96 open while this identity prerequisite and its existing remaining acceptance criteria are unmet. The independent panel and automatic-name fixes can proceed.
