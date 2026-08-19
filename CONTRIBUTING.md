# Contributing

Thanks for helping improve Miso MIDI. The project is still an early vertical
slice, so focused changes with explicit semantics and measurements are easier
to review than broad API additions.

## Development checks

Rust 1.97.1 and Python 3.10+ are supported. From the repository root:

```bash
uv sync --locked
uv run maturin develop --release
uv run pytest -q
uv sync --project benchmarks --locked
uv run --project benchmarks maturin develop --release
uv run --project benchmarks pytest -q benchmarks/tests
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build -p miso-midi-core --no-default-features
cargo check --manifest-path fuzz/Cargo.toml --bins
```

## Performance changes

Benchmark changes must keep the compared work equal and run the relevant
semantic preflight before timing. Include raw distributions and environment
metadata, disclose unstable runs, and retain negative results when they affect
the conclusion. Do not present diagnostic work-floor ratios as percentages of
theoretical performance.

The authoritative methodology is in
[the benchmark contract](benchmarks/docs/benchmark-contract.md).

## Pull requests

Keep pull requests scoped, describe observable behavior changes, and add tests
for malformed input and error offsets when parser behavior changes. Avoid
committing generated corpora, local benchmark outputs, virtual environments,
or build directories.
