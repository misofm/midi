# R2 score-layout evidence — 2026-08-19

This follow-up records the final R2 score-layout measurements. It supersedes
neither the method nor the earlier [baseline score report](python-score-baseline-2026-08-19.md):
both reports and their raw artifacts are retained so that changes in the result
are inspectable.

## Scope and method

The Python comparison parses already-loaded MIDI `bytes` into the Miso and
Symusic tick-score APIs. Before timing a dataset, it requires full equality
under `miso-score-contract/v1`: TPQ, ordered tracks and metadata, notes,
controls, bends, pedals, lyrics, time/key signatures, tempos, and markers.
Digesting and count checks are outside timing. The generated inputs use explicit
matched CC64 pedal pairs and exclude CC64 from their arbitrary-controller
stream.

The final pyperf run used 20 processes times 3 values, warm in-memory bytes,
and affinity to CPU 4. The Ryzen 7 9700X/Linux 6.8 host was non-isolated and
used the `powersave` governor. Raw distributions include warnings and outliers,
most notably in Symusic Mahler's mean; use medians below and inspect the raw
JSON rather than treating these values as noiseless constants.

| Dataset | Canonical digest | Miso median | Symusic median | Symusic/Miso |
| --- | --- | ---: | ---: | ---: |
| tiny | `bd36b66d…c9491acc` | 1.15893988 µs | 2.56139774 µs | 2.210x |
| normal | `d75cb3bb…559271f1` | 205.764829 µs | 374.545782 µs | 1.820x |
| huge | `fe10b416…7b12d37c` | 2.45459138 ms | 4.32661195 ms | 1.763x |
| Mahler | `d8fcfebd…009013f69` | 999.746531 µs | 1.77339135 ms | 1.774x |
| geometric mean | — | — | — | **1.883x** |

This is an equal-observable Python API result without file I/O. It is neither
a native 2x proof nor full Symusic-scope parity.

## Retained-score RSS: scoped gate passes

Fresh Linux subprocess workers retained scores at multiple checkpoints. The
slope includes native allocations, allocator behavior, and the preallocated
Python handle/list overhead. It is therefore an equal-Python-API process-RSS
proxy—not a pure native allocator measurement.

| Dataset | Score count | Miso slope | Symusic slope | Miso/Symusic | 50% gate |
| --- | ---: | ---: | ---: | ---: | --- |
| tiny | 8,192 | 1,204.00 B/score | 4,224.80 B/score | 28.50% | pass |
| normal | 64 | 166,272.00 B/score | 394,496.00 B/score | 42.15% | pass |
| huge | 64 | 2,020,070.40 B/score | 4,079,078.40 B/score | 49.52% | pass |
| Mahler | 64 | 989,516.80 B/score | 2,269,593.60 B/score | 43.60% | pass |

An independent huge repeat at count 128 was 2,019,660.8 versus 4,077,580.8
B/score (49.53%). Its raw artifact is intentionally transient; the retained
final artifacts are the primary evidence. The huge result clears the gate by
only about 0.47 percentage points, so it is a monitoring target rather than a
claim that the margin is stable across machines or allocators.

Miso's single-score Rust-owned `heap_bytes` values were 736 B (tiny), 165,408 B
(normal), 1,975,232 B (huge), and 1,142,350 B (Mahler). They are diagnostic
layout accounting and must not replace the RSS comparison.

## Native parser and diagnostic floors

The direct Rust harness consumes warm bytes, checks fixed corpus hash and
semantic cardinalities outside timing, and includes score destruction in the
timed operation. It records a distribution for `parse_score_smf` and separately
records byte-touch and one-contiguous-column allocation/write probes.

| Dataset | Native median | ns/byte | ns/event | input-touch ratio | alloc+write ratio |
| --- | ---: | ---: | ---: | ---: | ---: |
| tiny | 930.634 ns | 5.442 | 42.30 | 74.35x | 75.15x |
| normal | 207,295.785 ns | 1.975 | 12.659 | 21.81x | 98.81x |
| huge | 2,445,069.828 ns | 1.946 | 12.463 | 21.48x | 77.66x |
| Mahler | 1,002,897.125 ns | 1.528 | 10.321 | 16.86x | 59.42x |

The ratios are deliberately named comparisons with optimistic diagnostic
probes, not percentage-of-theoretical or hardware-efficiency claims. A real
parser must decode variable-length SMF structure, build score state, and manage
multiple allocations; one byte touch or one contiguous output write does not
model all of that work.

## Layout decision and ablations

R2 selected safe, transparent little-endian rows: `[u8; 10]` controls and
adaptive notes (`[u8; 10]` narrow, `[u8; 18]` wide). This avoids unsafe code,
packed-field access, and an ARM portability risk. Legacy note rows remain
lazily materialized.

The ablation record is intentionally short and negative results are retained:

- four-column SoA was slower;
- a direct two-column representation was slower;
- byte-packed rows recovered speed;
- representation-packed rows showed no material x86 gain and were rejected for
  portability;
- hot/cold separation plus open-note state produced the remaining win.

These observations motivate the selected layout for this vertical slice only;
they do not complete the broader score-model design or establish proximity to a
theoretical limit.

## What is now true, and what remains open

M1's scoped Python timing and retained equal-API RSS gates pass on this one
x86-64 run. M0 now has the canonical contract, raw Python timing, retained-RSS,
and native-floor evidence for this corpus. Sol independently ran formatting,
tests, clippy, no-default-feature checks, 62 Python tests, and Rust 1.97.1
tests/clippy/no-default checks for this revision.

The M1 milestone remains open: there is no native 2x-versus-Symusic competitor
measurement, no ARM benchmark, no full Symusic breadth or arbitrary-corpus
parity, and no completed resource-limit work. Those gaps also mean no claim of
theoretical-limit proximity is justified.

## Audited artifacts

- [score-r2-final.json](../results/score-r2-final.json) — pyperf
  distributions, contract metadata, corpus checksums, and output counts.
- [retained-score-memory-r2-tiny-final.json](../results/retained-score-memory-r2-tiny-final.json)
  — high-count tiny retained-RSS checkpoints.
- [retained-score-memory-r2-final.json](../results/retained-score-memory-r2-final.json)
  — normal, huge, and Mahler retained-RSS checkpoints.
- [native-score-r2-final.json](../results/native-score-r2-final.json)
  — native distributions, corpus/cardinality checks, machine metadata, and
  diagnostic-floor samples.

See [the benchmark contract](benchmark-contract.md), [the retained-memory
method](retained-score-memory.md), and [the native-score
method](native-score-benchmark.md) for exact boundaries and local reproduction
commands.
