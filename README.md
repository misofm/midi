# Miso MIDI

[![CI](https://github.com/misofm/midi/actions/workflows/ci.yml/badge.svg)](https://github.com/misofm/midi/actions/workflows/ci.yml)

Miso MIDI is a portable, high-performance MIDI core written in Rust. One
semantic implementation is designed to power native Rust applications,
Python, Node.js, and browser SDKs without forcing those SDKs into the same
object model. Rust and Python are implemented today; JavaScript remains on the
roadmap.

The repository is an early performance and architecture spike. The current
vertical slice offers an allocation-free structural scan, a compact owned
Standard MIDI File (SMF) arena, and a native-backed tick-score parser for the
checked score contract. The wire scanner is dependency-free and supports
`no_std`; owned arenas and the Python SDK add allocation explicitly.

## Why Rust

Parsing speed depends primarily on data layout, bounds checking, allocation,
and the amount of work performed across a language boundary. Rust and Zig can
both produce excellent machine code. Rust is the better project-level choice
because Miso Engine is already Rust and the same core can be consumed directly
there, while PyO3/maturin, napi-rs, and wasm-bindgen provide established SDK
paths. A generic C ABI remains an optional adapter for hosts that need it.

## Repository shape

```text
crates/miso-midi-core/   dependency-free Rust parser primitives
bindings/python/         thin PyO3 extension
python/miso_midi/        ergonomic Python package
benchmarks/              generated corpus and pyperf suite
docs/                    architecture and product scope
```

## Develop

Rust 1.97.1 and Python 3.10+ are required. `uv` installs Python development
dependencies and builds the native extension through maturin.

```bash
uv sync
uv run maturin develop --release
uv run pytest
uv run --project benchmarks python -m benchmarks.corpus
uv run --project benchmarks pytest -q benchmarks/tests
cargo test --workspace
cargo build -p miso-midi-core --no-default-features
```

Run a short benchmark:

```bash
mkdir -p benchmarks/results
uv run --project benchmarks python -m benchmarks.bench_parse \
  --fast \
  --output benchmarks/results/local.json

uv run --project benchmarks python -m benchmarks.bench_malformed \
  --fast \
  --output benchmarks/results/malformed.json

# Equal-observable tick-score construction against the benchmark reference.
# This preflights canonical equality before it times either side.
uv run --project benchmarks python -m benchmarks.bench_score \
  --fast \
  --output benchmarks/results/score-local.json
uv run --project benchmarks python -m benchmarks.summarize_score \
  benchmarks/results/score-local.json

# Native Rust score-parser distributions and diagnostic work floors.
# Choose CPU affinity externally; this command emits raw JSON, not a headline.
taskset -c 4 cargo run -p miso-midi-native-score-bench --release -- \
  --output benchmarks/results/native-score-local.json

# The source-pinned native comparison is documented with its equal-work
# preflight and limitations in benchmarks/docs/.
```

Use the Python SDK from bytes:

```python
from miso_midi import ScoreParseLimits, parse, parse_score

data = open("song.mid", "rb").read()
midi = parse(data)
print(midi.track_count, midi.event_count, midi.heap_bytes)

# Native-backed tick score; semantic_records() is a bulk diagnostic/export
# surface, not part of the fast parse call.
score = parse_score(data)
print(score.semantic_records()["tpq"])

# parse_score is finite by default for untrusted bytes. Customize a ceiling
# explicitly when an application has a smaller admission budget.
score = parse_score(data, limits=ScoreParseLimits(max_events=100_000))
```

`parse_score_unlimited(data)` is a trusted-input legacy escape hatch and can
exhaust memory on hostile bytes. See the [score-parser resource policy](benchmarks/docs/score-parser-resource-policy.md)
for defaults, strict mode, and error offsets.

The benchmark reports compact-arena and reference object-graph parse times
separately. It also converts both results into identical semantic Python tuples
for an equal-output end-to-end comparison.

N6 measures the finite-default Compatible Python API against the reference and
a separate trusted native path against pinned reference source. Its accepted
performance-governor ABBA run reaches a 2.490x native median geometric mean;
tiny remains below the per-dataset 2x target. The Python checked-default median
geometric mean is 2.463x, with trusted-path overhead reported separately. This
is neither full reference-scope parity nor a theoretical-limit claim. The scoped
retained-RSS gate passes but huge is narrow. Read conditions, raw artifacts,
and open gates in [the N6 evidence report](benchmarks/docs/native-n6-evidence-2026-08-19.md).
N5, R2, and baseline reports remain historical evidence.

## Direction

The intended public product is broader than an SMF parser:

- lossless and normalized SMF read/write modes;
- an incremental, bounded live MIDI 1.0 stream decoder;
- MIDI 2.0 UMP types and conversion where semantics are well-defined;
- borrowed event views plus an optional owned arena;
- bulk columnar access for Python/NumPy and JavaScript typed arrays;
- native Rust, Python, Node, browser/Wasm, and optional C-ABI adapters;
- fuzzing, differential tests, corpus conformance, and transparent benchmarks.

See [docs/architecture.md](docs/architecture.md) and the
[benchmark contract](benchmarks/docs/benchmark-contract.md). The staged
benchmark program is in [the research roadmap](benchmarks/docs/research-roadmap.md).
Current comparative results are the [Python event report](benchmarks/docs/python-event-comparison-2026-08-19.md),
[N6 safe-default report](benchmarks/docs/native-n6-evidence-2026-08-19.md),
[N5 native report](benchmarks/docs/native-n5-evidence-2026-08-19.md), and
[R2 score report](benchmarks/docs/score-layout-r2-2026-08-19.md). The
[retained-score-memory method](benchmarks/docs/retained-score-memory.md) is
published separately from timing. The direct-Rust parser and diagnostic floor
method is in [the native-score benchmark](benchmarks/docs/native-score-benchmark.md).
The source-pinned native comparison design is documented in
[the native comparison method](benchmarks/docs/native-comparison.md).

## License

Licensed under the [Apache License 2.0](LICENSE).

Contributions are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md) for the
development and benchmark-integrity checks used by this repository.
