# Native Miso versus Symusic score construction

This harness is the native-comparison path for the tick-score vertical slice.
It exists because a Python API timing result is not evidence about the native
Rust and C++ parser kernels.

It compares warm, already-loaded corpus bytes in separate native binaries:

```text
miso_midi_core::parse_score_smf(bytes)              # Rust
symusic::Score<Tick>::parse<DataFormat::MIDI>(span) # C++
```

Each operation constructs its score, feeds it to a compiler barrier, then lets
the local score destruct before the next operation. File I/O, corpus hashing,
semantic extraction, and the preflight are outside timing. The binaries produce
raw per-operation distributions; the merger derives ratios only after every
contract check passes.

## Pinned upstream surface

The competitor is fetched as source from
[`Yikai-Liao/symusic`](https://github.com/Yikai-Liao/symusic) commit
[`43ff25277abbc72dbd8d00fb5a9a14ec37fb7906`](https://github.com/Yikai-Liao/symusic/tree/43ff25277abbc72dbd8d00fb5a9a14ec37fb7906),
which is the `v0.6.0` tag (2026-04-08). The later roadmap commit `3cdd0ee` is
`v0.6.0-787-g3cdd0ee` and is deliberately not used. No wheel or opaque prebuilt
competitor binary is vendored.

The Python preflight remains pinned to `symusic==0.6.0` in
`benchmarks/pyproject.toml`, and
the lock resolves the 2026-04-08 source distribution
`2290a4dd8adb77e6f9b66b75ee47182e426f63d34d83b8741bccc6f8bb49ceae`.
For the CPython 3.12 manylinux x86-64 wheel used by this environment,
`benchmarks/uv.lock`
records `78b3799d99f662f16f7973b38492ead1dbf4acdff129788b560a183c2802581a`.
The source-built C++ competitor is nevertheless the authoritative native side
of this benchmark.

The native entry point is public C++ API, not a Python binding shortcut:

- [`include/symusic/score.h`](https://github.com/Yikai-Liao/symusic/blob/43ff25277abbc72dbd8d00fb5a9a14ec37fb7906/include/symusic/score.h)
  declares `Score<T>::parse<DataFormat::MIDI>(std::span<const u8>)` and exposes
  score-level fields.
- [`include/symusic/io/midi.h`](https://github.com/Yikai-Liao/symusic/blob/43ff25277abbc72dbd8d00fb5a9a14ec37fb7906/include/symusic/io/midi.h)
  declares the explicit MIDI specializations.
- [`src/io/midi.cpp`](https://github.com/Yikai-Liao/symusic/blob/43ff25277abbc72dbd8d00fb5a9a14ec37fb7906/src/io/midi.cpp)
  implements the in-memory parser and populates notes, controls, bends,
  pedals, lyrics, time/key signatures, tempos, and markers.

`benchmarks/native_symusic/fetch_symusic.sh` clones into an external cache,
checks out that exact detached commit, initializes submodules recursively, and
checks every top-level Symusic gitlink before CMake builds the benchmark. It
also rejects tracked or untracked changes in the root checkout and every
recursive submodule. CMake independently resolves `git rev-parse HEAD` and
fails unless it is exactly `43ff252…7906`. It configures a fresh `-G Ninja`
cache, requires a Release build, verifies IPO support, requires IPO on both the
benchmark executable and upstream `symusic` target, and records compiler,
CMake version/generator, flags, build type, and IPO state. The cache can be relocated with
`MISO_SYMUSIC_CACHE_DIR`; it is never part of the repository or a production
dependency. `uv sync --project benchmarks` supplies the pinned CMake, Ninja,
and Maturin build tools;
a C++20 compiler, Git, and a Linux `taskset` benchmark host are still required.

## Equal-work preflight

The C++ harness reproduces `miso-score-contract/v1`'s length-prefixed binary
encoding and SHA-256 directly from `Score<Tick>` public fields. Before and after
timing it requires each corpus input hash, full canonical semantic digest, and
all cardinalities to equal the fixed M0 values. It therefore fails if a source
or API change alters a visible tick-score result.

`benchmarks.native_symusic.preflight` additionally constructs Miso and Symusic
through their public Python score APIs before native timing and requires both to
match the same fixed full digest and counts. Both native raw harnesses now
independently compute the same complete digest through their public score
getters before and after timing. `combine.py` refuses to produce a ratio unless
the preflight and both native full-digest/count reports agree.

Generated corpora contain matched CC64 on/off pairs and deliberately exclude
CC64 from their arbitrary-controller sequence. This avoids comparing accidental
unmatched-pedal behavior.

## Reproduce on a benchmark host

Run this only on a prepared Linux host; it creates local raw outputs and should
not overwrite checked-in final artifacts.

```bash
MISO_NATIVE_AFFINITY=4 \
MISO_NATIVE_OUTPUT_DIR=benchmarks/results/native-score-local \
bash benchmarks/native_symusic/run_native_compare.sh
```

The runner rebuilds the local Miso extension for the public preflight, fetches
and verifies source-pinned Symusic, then runs an ABBA sequence (Miso, Symusic,
Symusic, Miso) under the same requested affinity. It writes:

- `preflight.json` — fixed full digest/count agreement outside timing;
- `miso-a.json`, `miso-b.json` — Rust raw distributions and machine metadata;
- `symusic-a.json`, `symusic-b.json` — C++ raw distributions, compiler/source
  pin/build flags/IPO state, and machine metadata;
- `comparison.json` — samples pooled only after it proves exact Symusic commit,
  input bytes/hashes, full digests/counts, release/non-debug state, identical
  sample configuration, identical CPU model/affinity/governor/kernel state, and
  no more than 5% A/B raw-run median drift per implementation/dataset. It includes each
  per-dataset ratio and their geometric mean.

The two implementations use separate processes so their allocator/import state
is not shared. The ABBA ordering reduces one-sided warm/drift bias but does not
make a non-isolated host trustworthy by itself. A serious claim still needs a
controlled host, repeated runs, published governor/frequency state, and all
raw artifacts. The binaries record Linux affinity, CPU model, selected first
affinity CPU's governor, and kernel release, but do not modify a governor.

## Validation and non-claims

The CMake target has a `--self-test`/CTest target for the SHA implementation and
Linux metadata parser. Both the C++ and Rust harnesses offer `--verify-only` to
validate all fixed corpus/contract values without timing. Rust additionally
offers `--parse-only`/`--no-floors`; the comparison runner uses it so diagnostic
floor probes never become asymmetric timed work. Neither verification path is a
headline benchmark.

N6 publishes the accepted safe-default-era native result: a 2.490x pooled-median
geometric mean on the checked corpus and one non-isolated x86-64 host. Its
aggregate 2x gate passes, but tiny is 1.616x, so no per-dataset-uniform 2x
claim is made. A powersave candidate was rejected by the new drift gate. See
the [N6 evidence report](native-n6-evidence-2026-08-19.md) and raw artifacts for
distributions and full conditions. ARM64, arbitrary-file universality, and full
Symusic breadth remain open.
