# Owned offline LocalEngine comparison — 3 November 2026

> The linked JSON receipt is a sanitized public derivative. Owner-specific generation/model/log paths are redacted; historical revisions, artifact hashes, measurements and limitations are unchanged.

This exploratory microbenchmark includes the byte-identical LocalEngine modules from baseline `340d02e` and candidate `5d63a37`. It uses one newly generated, owned English TTS WAV (3.989 seconds, 16 kHz mono PCM16) and the already installed Whisper Large v3 Turbo artifact. The source wrapper removes only serialization/FFI derives from timing values, adapts unrelated error/Apple ARM availability types, and adds a read-only preview-registration barrier. This is not a packaged Pennel test.

No model was downloaded, no hosted request was made and no app settings, providers, credentials or active data were read or changed. The preinstalled Aman TTS voice provided the fixture. The then-committed `hello.wav` at this comparison’s source revisions was not used because its originating commit did not establish recording consent or synthetic provenance. This historical caveat does not describe replacement fixtures added later.

| Observed measure | Baseline | Candidate |
|---|---:|---:|
| Warm authoritative median, 10 samples each in serial ABBA order | 649.1 ms | 656.0 ms |
| Warm authoritative range | 572.4–778.4 ms | 604.6–674.6 ms |
| Numeric reference edit distance, every completion | 1 of 13 words | 1 of 13 words |
| Retained authoritative timing spans | 0 | 2 real Whisper segment spans |
| Sampled process-tree peak RSS, paired runs | 176,586,752 bytes | 178,798,592 bytes |

Three current warm previews over six-second buffers completed in 592.4–615.4 ms. Foreground demand cancelled all three separate active previews. Observed demand-to-preview-return was 455.5–477.7 ms; authoritative completion including that physical wait was 1080.2–1122.1 ms. Cancellation is logical before native decoding returns; these measurements must not be described as instant physical cancellation. The harness executed 33 authoritative and six preview Session.run calls, including three cancelled previews.

The first new-engine baseline call took 5125.5 ms; the later new-engine calls took 712.4–794.1 ms. OS and GPU caches were not reset, so this is not a controlled cold-start comparison. Model SHA-256 was unchanged before/after: `b2e30cc286bc9f3aba4db9099fc7403543497c05ce7100d0d83091ddfd25a183`. Both versions use the production-default native Auto policy with Metal and CPU compiled; the actual LocalEngine-owned Session backend was not separately queried.

Concurrent native build processes ranged from 14 to 34, and sampling itself adds overhead. These paired timings do not establish a regression or improvement. Process-tree RSS includes the benchmark and its time wrapper only; it excludes driver/GPU allocations and the Pennel app. Per-run CPU and resident/footprint counters, raw numeric events, source/binary/dependency hashes and commands with machine-identifying components redacted are in [receipt.json](receipt.json). Transcript content is absent from logs.

No latency, resource, quality or background activation gate is passed. Whole-app input/Stop latency, two physical tracks, live/final attribution quality, Dutch/Flemish/mixed content, dictation contention, long sessions and signed-device behavior remain unmeasured. Compatible speaker segmentation/embedding model assets are absent from Pennel’s normal installed model directory; none were acquired.

Reproduce only with the recorded installed artifact: `"$HOME/.local/bin/worktree-storage" run -- python3 -B build/notetaker-2026-11-03/owned-comparison/run.py`. This replaces only the ignored benchmark’s per-run numeric output and receipt, and never changes the product.
