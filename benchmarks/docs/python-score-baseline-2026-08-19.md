# Miso MIDI versus Symusic score construction — 2026-08-19

## Scope of this result

This is the first Miso/Symusic equal-observable score-construction comparison.
It measures warm, already-loaded MIDI `bytes` passed through the Python APIs:

```python
miso_midi.parse_score(data)
symusic.Score.from_midi(data)
```

The timed calls construct native-backed tick-score objects. File I/O, contract
digesting, and conversion to Python semantic records are outside the timed
region. Before each dataset is timed, the harness constructs both scores and
requires equality under `miso-score-contract/v1`; a mismatch or count mismatch
aborts the entire run.

This is a Python API result, not a proof of a 2x native Rust-kernel advantage.
It is also not a claim of full Symusic scope parity: time-domain conversion,
editing, piano rolls, serialization, ABC, and synthesis remain outside this
vertical slice. It says only that the two observed tick-score results agreed for
the generated/checksum-pinned local corpus and that the shown Python entry
points were timed without I/O.

## Contract and corpus

The canonical contract includes TPQ; ordered track metadata; notes, controls,
pitch bends, pedals, lyrics; and global time/key signatures, tempos, and
markers. Its binary digest and output counts are calculated before timing.
Generated tracks contain one explicit matched CC64 sustain on/off pair and the
periodic controller stream excludes CC64; no unmatched-pedal behavior is used
to obtain equality.

| Dataset | Input bytes | Canonical SHA-256 | Observable summary |
| --- | ---: | --- | --- |
| tiny | 171 | `bd36b66d…c9491acc` | 1 track; 16 notes; 3 CC; 1 bend; 1 pedal; 1 marker |
| normal | 104,974 | `d75cb3bb…559271f1` | 8 tracks; 16,000 notes; 272 CC; 64 bends; 8 pedals; 32 markers |
| huge | 1,256,430 | `fe10b416…7b12d37c` | 16 tracks; 192,000 notes; 3,040 CC; 752 bends; 16 pedals; 384 markers |
| Mahler | 656,425 | `d8fcfebd…009013f69` | 51 tracks; 60,411 notes; 36,287 CC; 97 time/key signatures; 177 tempos; 97 markers |

The full corpus checksums, canonical hashes, and every individual output count
are embedded in the raw result metadata.

## Environment and method

- AMD Ryzen 7 9700X; Linux 6.8.0-138; CPython 3.12.3
- Rust 1.97.1; release extension with thin LTO
- Miso MIDI 0.1.0; Symusic 0.6.0; pyperf 2.10.0
- warm in-memory bytes; default pyperf 20 processes × 3 values (60 values per
  named benchmark); `--affinity 4`
- the host was **not isolated** and CPU 4 used the `powersave` governor

Consequently, the distributions—not only their medians—matter. The raw data
contains CPU frequency/load metadata and every sample. In particular, tiny has
unstable/outlying samples for both libraries and should be treated as a fixed
cost signal, not a throughput result.

## Parse-score medians

Smaller is better. “Speedup” is Symusic median divided by Miso median.

| Dataset | Miso median | Symusic median | Speedup |
| --- | ---: | ---: | ---: |
| tiny | 0.864115 µs | 2.585142 µs | 2.992x |
| normal | 207.424 µs | 372.449 µs | 1.796x |
| huge | 2.508272 ms | 4.322114 ms | 1.723x |
| Mahler | 1.028494 ms | 1.753666 ms | 1.705x |
| geometric mean | — | — | **1.993x** |

The geometric mean of per-case means is **2.006x**. These are useful M1
vertical-slice measurements, but they do not meet the roadmap's claimed native
2x gate because they are Python API timings, and the tiny result is not stable
enough to promote as a standalone claim.

## Diagnostic only: normal-case `perf stat`

After the final build, CPU 4 was profiled for 10,000 normal-score parses. This
is not a pyperf result: the counters were multiplexed at 83% and the profiling
task times (228 µs Miso; 412 µs Symusic) are not interchangeable with the
pyperf medians above. Treat it as a direction-setting signal, not a floor or
release claim.

| Approximate per parse | Miso | Symusic |
| --- | ---: | ---: |
| instructions | 4.037 M | 9.178 M |
| cycles | 1.261 M | 2.227 M |
| branches | 819 K | 1.680 M |
| branch misses | 739 | 6,062 |
| cache misses | 737 | 1,676 |

The sample suggests that the next work should inspect allocation/layout and
memory/IPC behavior, then validate any change with isolated floor and counter
audits. It does not establish a final microarchitectural explanation.

## Retained-memory result: gate not met

Retained-score memory was measured independently in fresh Linux subprocesses
using current RSS (`/proc/self/statm`) over multiple retained-score checkpoints.
The slope includes incremental native/proxy allocations; the raw reports also
include the preallocated Python score-handle list and its per-slot overhead.
This is a process-RSS estimate, not a portable allocator-heap measurement.

| Dataset | Miso RSS slope | Symusic RSS slope | Miso versus Symusic | 50%-of-Symusic gate |
| --- | ---: | ---: | ---: | --- |
| tiny | 1,636.71 B/score | 4,282.55 B/score | -61.8% | pass |
| normal | 400,490.99 B/score | 394,383.37 B/score | +1.55% | fail |
| huge | 4,728,993.57 B/score | 4,076,806.73 B/score | +16.0% | fail |
| Mahler | 2,340,219.18 B/score | 2,272,613.34 B/score | +2.97% | fail |

The overall memory gate is **not met**. The retained RSS slopes for normal,
huge, and Mahler are not within the roadmap target of at most 50% of Symusic.
Do not substitute Miso's single-score `heap_bytes` metric for this comparison:
it is a Rust-owned heap counter, not process RSS and not an equivalent Symusic
metric. Its observed one-score values were 1,176 B (tiny), 400,256 B (normal),
6,364,928 B (huge), and 3,102,976 B (Mahler).

## Reproduce and inspect

The following audited raw artifacts are intentionally trackable in the
repository; transient benchmark outputs remain ignored:

- [score-final.json](../results/score-final.json) — pyperf samples,
  timing metadata, corpus checksums, contract hashes, and output counts.
- [retained-score-memory-tiny-final.json](../results/retained-score-memory-tiny-final.json)
  — high-count tiny RSS slope.
- [retained-score-memory-final.json](../results/retained-score-memory-final.json)
  — normal, huge, and Mahler RSS slopes.

To produce a new local result rather than overwrite these release artifacts:

```bash
mkdir -p benchmarks/results
uv run --project benchmarks python -m benchmarks.bench_score \
  --affinity 4 \
  --output benchmarks/results/score-local.json
uv run --project benchmarks python -m benchmarks.summarize_score \
  benchmarks/results/score-local.json

uv run --project benchmarks python -m benchmarks.measure_score_memory \
  --datasets tiny \
  --count 4096 \
  --output benchmarks/results/retained-score-memory-tiny-local.json
uv run --project benchmarks python -m benchmarks.measure_score_memory \
  --datasets normal huge mahler \
  --count 32 \
  --output benchmarks/results/retained-score-memory-local.json
```

Local results should report their own environment and must not be merged with
the values above. See [the benchmark contract](benchmark-contract.md) and
[the retained-memory method](retained-score-memory.md) for the
measurement boundaries.
