# Core speech fixture

`hello.wav` is generated English speech used by provider connection checks and
local-recognition tests. It contains the original test sentence:

> Hello. This is a Pennel speech test.

The fixture is PCM16 WAV, mono, 16 kHz. Its canonical generation commands,
tool versions, voice identity, format and checksum are recorded in
[`hello.provenance.json`](hello.provenance.json). Run those commands from the
repository root to reproduce it with eSpeak NG 1.52.0 and FFmpeg 9.0.2.

Generation uses eSpeak NG's standard `en-us` formant voice (`gmw/en-US`),
without MBROLA or recorded speech. The generator is an external development
tool, not a Pennel application dependency. The fixture is original synthetic
test material distributed under this repository's Apache-2.0 license.
eSpeak NG's own GPL-3.0-or-later license and source are identified in the
provenance record; no generator executable or source code is bundled here.

For an offline recognition check, first install Whisper Turbo through the
normal application workflow, then run only this test:

```sh
scripts/with-toolchain.sh cargo test --manifest-path core/Cargo.toml --features local-models --lib local_engine::tests::real_existing_whisper_turbo_fixture_transcribes -- --ignored --exact
```

The test refuses to run if the model is absent and never downloads one. It
requires the generated English transcript to contain `test`. Do not run the
whole ignored suite for this check: other tests have different model or
download requirements.

Historical receipts retain their own source revisions and artifact hashes.
This replacement does not establish the provenance of an earlier recording
or revalidate older benchmark results.
