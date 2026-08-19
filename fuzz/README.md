# Fuzzing Miso MIDI

This is a standalone `cargo-fuzz` package. It is intentionally outside the
root workspace so libFuzzer settings do not affect ordinary builds.

Install the runner and a nightly toolchain once:

```sh
cargo install cargo-fuzz
rustup toolchain install nightly
```

Run the raw API target or the deterministic valid-SMF target:

```sh
cargo +nightly fuzz run smf_apis
cargo +nightly fuzz run structured_score
```

`smf_apis` sends at most 64 KiB of arbitrary bytes through `scan_smf`,
`parse_smf`, trusted `parse_score_smf`, and a deliberately small checked
`parse_score_smf_with_limits` policy, plus finite Compatible and Strict
`parse_score_smf_with_options` policies. The cap keeps the trusted parser's
unbounded allocation behavior out of long-lived fuzz workers while still
exercising malformed, resource-limit, and grammar-policy paths.

`structured_score` maps input bytes into a small valid SMF and asserts that
the trusted score parser agrees exactly with the legacy unlimited-limits
parser and finite Compatible and Strict option policies.

For a compile-only check that does not need `cargo-fuzz` installed:

```sh
cargo check --manifest-path fuzz/Cargo.toml --bins
```

Fuzzer corpora, crash artifacts, coverage output, and target output are
ignored by `fuzz/.gitignore`. Do not treat fuzzing as proof of arbitrary-file
universality; promote minimized crashes and durable regressions into ordinary
deterministic tests.
