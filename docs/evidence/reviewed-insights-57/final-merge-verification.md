# Final joined merge verification

The reviewed production integration combines physical recovery-audio provenance from #128/#129 with #132's exact receipt-body guard. The original guard commit is `878008a`; its complete source file remains SHA-256 `562d14a62fc282bcca024f6748bd4e09b4c16826d9cea1ce2874c97ef0fb290a`.

Current-production verification ran on `a496495e75f6af327f80ab6429713ddb759c4b88`. The final test-only parent update rebased onto #131 `e00bb9ac2b4adcc758ab01de94e7338d6179d19b`, yielding `360d5d2b6289e9de1f0e0e5abd1fbe931d312aad` before this evidence commit. Production code, native inputs and the receipt guard are identical across that update. The repaired spool fixture passed as an exact test in default and lean configurations; no second broad/native/Release loop was run.

The [machine receipt](final-merge-verification.json) records the six reviewed audio-prefix hashes, original guard hash, current native products and result summaries. All eleven pre-existing review-phase edits are byte-identical. Reversing those edits in an isolated Git index reconstructs the exact #98 tree; reversing the assistance phases reconstructs the exact reviewed #129 tree. This preserves the audio fix through later `engine.rs` and `lib.rs` additions without claiming their complete phase-specific prefixes are identical.

| Required check | Result |
| --- | --- |
| Full default core / lean core | 1002 passed + 7 ignored / 985 passed + 5 ignored |
| Core release; strict default/lean/evaluation Clippy; formatting | Passed |
| Evaluation feature / keyboard / speaker ONNX | 10 / 17 / 15 passed + 1 ignored |
| Desktop frontend tests/build / backend tests and strict Clippy | 291 + build passed / 258 passed |
| Mac Swift / mobile helper tests | 139 / 116 passed |
| Mobile type/UI/plugins/widgets/pods | Passed / 121 / 17 / 7 / 6 passed |
| Current rebuilt Rust libraries and Swift bindings | Both ARM64 targets passed; public binding hash unchanged |
| Current-library native tests | 171 passed, 0 failed, 0 skipped |
| Current-library Release builds | Simulator then iPhoneOS ARM64 passed sequentially |
| Offline evaluation / cached artifact audit | 287 passed / four existing artifacts verified without loading or downloading |
| Offline prefix replay | 4500 cases, 12 candidates, 0 adopted records |
| Packaging / theme / fixture helpers | 61 / passed / 34 passed |

The generated mobile project and binding shape were unchanged, so the previously verified clean/repeated Prebuild was reused and pod checks rerun. Native verification used newly built matching Rust libraries. Debug test signing was explicitly ad hoc with an empty team; Release signing was disabled. Fresh certificate preflight confirmed personal team `B538Q2K76C`, SHA-1 `26AD2732047AB5E2F5CB1B239B71806C54EB3026`; no employer identity was used. Prior apps, dSYMs, core libraries, logs, screenshots and result bundles remain retained.

The original screenshots and baseline measurements retain their original candidate scope; UI source did not change and no additional capture matrix was needed. The format documentation now describes schemas 1/3 for settled attribution and 2/4 for provisional attribution without/with recovery proof, including older-reader preservation and retention of completion provenance during refinement.

Qualification remains empty and live/background #97 stays disabled. These checks support source integration and merging, while real-call/acoustic quality, resource, signed physical-device, keyboard/store and deployment acceptance remain open.
