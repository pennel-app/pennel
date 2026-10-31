# Pennel typing policy

This internal crate supplies stateless, bounded local typing decisions. It does not construct a Pennel engine, open a database, retain a document, contact a provider, or log input. The main core consumes the Rust API with default features disabled. The keyboard links the standalone archive with its default `ffi` feature enabled.

`suggest` combines the current token, active `en`/`nl` languages, native spelling/completion candidates, explicit personal words, optional word frequencies and learned next-word counts. Scores express relative ranking utility, not probabilities. Automatic correction is limited to authored common typo mappings and is blocked by bilingual validity, personal vocabulary, protected tokens, disabled correction, or a rejected replacement. Snippets use exact triggers and require a separate word-boundary expansion opt-in. Document identity, cursor validation, replacement and immediate undo belong to the native caller.

`decode_swipe` compares a bounded path with candidate key-center paths. All coordinates must use the same space; exact key centers allow QWERTY and AZERTY without hard-coded screen geometry. The authored common-word vocabulary is a fallback, not a trained language model or a comprehensive dictionary. Callers can supply additional bounded vocabulary. Swipe output always requires the native caller's explicit commit and should remain experimental until real-device quality and latency acceptance.

The C interface in `include/` accepts borrowed UTF-8 JSON bytes with an explicit length and returns a Rust-owned JSON C string. Release each response exactly once with `pennel_typing_free_response`. Null/empty inputs, malformed JSON, invalid languages, nonfinite geometry, unknown fields and excessive bounds fail closed with content-free error codes. A nonnull input pointer must address at least the given number of readable bytes; the caller retains ownership throughout the call. Never pass an arbitrary pointer or free a response twice.

Limits: 1 MiB serialized input; 128 UTF-8 bytes and 64 characters per token/word; 64 native candidates; 4,096 supplied vocabulary entries; 2,048 personal words; 512 learned next-word entries; 128 snippets with 2,048-byte replacements; 256 path/touch points; 64 distinct key centers; 1–8 output candidates. Optional timestamps are monotonic milliseconds relative to the current gesture. Requests exceeding a bound are rejected rather than silently truncated.

The Swift facade and local pod live in `apps/mobile/native/ios/Typing`. Build packaging supplies `PennelTypingFFI.xcframework` outside generated native projects. Basic typing must remain available if a suggestion request fails.

Verification:

```sh
scripts/with-toolchain.sh cargo test --manifest-path core/keyboard/Cargo.toml
scripts/with-toolchain.sh cargo test --manifest-path core/keyboard/Cargo.toml --no-default-features
scripts/with-toolchain.sh cargo clippy --manifest-path core/keyboard/Cargo.toml --all-targets -- -D warnings
```
