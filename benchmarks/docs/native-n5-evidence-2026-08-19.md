# Native Miso versus exact Symusic v0.6.0 — N5 evidence (2026-08-19)

This report records the first native, equal-work Miso-versus-Symusic result.
It preserves the earlier [R2 Python/API result](score-layout-r2-2026-08-19.md)
and the initial hardened N3 native result as historical evidence; it does not
claim full Symusic scope parity, arbitrary-file universality, or closeness to a
theoretical hardware limit.

## What was compared

The Rust side calls `miso_midi_core::parse_score_smf(bytes)`. The C++ side
calls `symusic::Score<Tick>::parse<DataFormat::MIDI>(span)` from the exact
Symusic `v0.6.0` commit
[`43ff25277abbc72dbd8d00fb5a9a14ec37fb7906`](https://github.com/Yikai-Liao/symusic/tree/43ff25277abbc72dbd8d00fb5a9a14ec37fb7906).
Both consume already-loaded bytes, construct a score, feed it to a compiler
barrier, and destroy it inside the timed operation. File I/O, hashing, count
checks, and semantic-record construction are outside timing.

Both the timed Rust `parse_score_smf` and Python `parse_score` calls use the
trusted/unlimited path. Checked Rust limits exist, but they are not the path
timed here; this report makes no hostile-input safety or bounded-resource claim.

Before timing, the Python preflight and both native binaries independently
require the complete `miso-score-contract/v1` digest and cardinalities. The
contract covers TPQ; ordered tracks and metadata; notes, controls, pitch bends,
pedals, lyrics; and global time/key signatures, tempos, and markers. The four
checked digests are tiny `bd36b66d…c9491acc`, normal `d75cb3bb…559271f1`, huge
`fe10b416…7b12d37c`, and Mahler `d8fcfebd…009013f69`. Generated files contain
only valid matched CC64 pedal pairs.

The native runner uses separate processes in ABBA order (Miso, Symusic,
Symusic, Miso). Each raw process collected 30 samples after five warm-ups with
automatic iteration calibration to a 50 ms minimum; the fail-closed merger
pooled the two raw runs into 60 samples per implementation and dataset only
after exact source, input, contract, Release/non-debug/IPO, configuration, CPU
model, affinity, governor, and kernel conditions matched. The C++ harness was
built from source with UV-managed CMake/Ninja, Release and IPO; Miso used the
workspace release profile (`thin` LTO, one codegen unit, `panic=abort`).

Environment: AMD Ryzen 7 9700X, Linux 6.8.0-138, CPU 4, `powersave` governor,
Rust 1.97.1. The host was not isolated. The native pooled distributions have
CVs below 0.9%, which is cleaner evidence than the supplementary Python run,
but a governor-controlled isolated-host repeat is still required.

## Native result

Smaller is better; speedup is Symusic pooled median divided by Miso pooled
median.

| Dataset | Miso median | Symusic median | Speedup |
| --- | ---: | ---: | ---: |
| tiny | 905.640 ns | 1,478.500 ns | 1.633x |
| normal | 114.781 µs | 349.668 µs | 3.046x |
| huge | 1.325 ms | 4.114 ms | 3.104x |
| Mahler | 672.059 µs | 1.659 ms | 2.469x |
| geometric mean | — | — | **2.485x** |

The aggregate native 2x geometric-mean gate **passes** on this corpus and
machine. The per-dataset 2x gate **fails for tiny** (1.633x), so this is not a
uniform 2x claim. It also does not carry over to untested formats, operations,
or platforms.

For transparent ablation context, the retained initial hardened N3 raw run had
a 1.398x geometric mean. N3-to-N5 Miso median changes were 1.186x (tiny),
2.238x (normal), 2.263x (huge), and 1.771x (Mahler), a 1.806x geometric mean.
Competitor medians drifted slightly between runs, so those are cross-run
diagnostics about Miso's own change, not a same-run competitive ratio.

## Supplementary Python and retained-RSS evidence

The equal-observable Python API pyperf run (20 processes × 3 values, warm
bytes, affinity 4) produced these medians:

| Dataset | Miso median | Symusic median | Speedup |
| --- | ---: | ---: | ---: |
| tiny | 1.126126 µs | 2.597603 µs | 2.307x |
| normal | 114.339 µs | 383.305 µs | 3.352x |
| huge | 1.317011 ms | 4.346878 ms | 3.301x |
| Mahler | 667.133 µs | 1.764922 ms | 2.646x |
| geometric mean | — | — | **2.867x** |

Treat this as supplemental only. Symusic tiny and especially normal are
bimodal; normal's raw mean/stddev is 468 µs / 124 µs. Therefore this report
does not headline its 3.062x mean geometric mean or treat pyperf medians as
cleaner than the native ABBA evidence.

Fresh-process retained-score RSS slopes include native allocations, allocator
behavior, Python proxies, and preallocated handle/list overhead. They are an
equal-Python-API Linux RSS proxy, not a pure native allocator measurement.

| Dataset | Miso B/score | Symusic B/score | Miso/Symusic | ≤50% gate |
| --- | ---: | ---: | ---: | --- |
| tiny | 1,198.00 | 4,258.39 | 28.13% | pass |
| normal | 166,106.53 | 395,288.47 | 42.02% | pass |
| huge | 2,019,929.11 | 4,081,904.96 | 49.48% | pass |
| Mahler | 990,988.63 | 2,269,317.64 | 43.67% | pass |

The scoped RSS gate passes, but huge clears it by only 0.52 percentage points;
repeat it on another allocator/machine before treating the margin as durable.

## Native floor diagnostics

The separate native Miso harness includes score destruction, validates full
contract metadata outside timing, and reports optimistic diagnostic probes. It
is not a competitor comparison and does not establish a theoretical limit.

| Dataset | Parse median | ns/byte | ns/event | byte-touch ratio |
| --- | ---: | ---: | ---: | ---: |
| tiny | 906.452 ns | 5.301 | 41.202 | 59.01x |
| normal | 117.969 µs | 1.124 | 7.204 | 12.43x |
| huge | 1.395 ms | 1.111 | 7.113 | 12.27x |
| Mahler | 673.741 µs | 1.026 | 6.934 | 11.35x |

For substantive inputs, this is 1.03–1.12 ns/byte and 6.93–7.20 ns/event;
the byte-touch ratios are 11.35–12.43x. A byte touch and contiguous allocation/
write probe omit variable-length decoding, score state, and multiple
allocations, so these ratios are diagnostic comparisons, never a percentage of
theoretical performance.

## What changed in N5

The measured implementation adds a zero-sized unlimited/stateful bounded
policy, a force-inlined `add_note`, and a scalar score-only channel decoder.
The core retains checked resource limits and running-status cancellation. The
opt-in exact-v0.6.0 testcase corpus also has full equality for 27 files totaling
1,717,487 bytes; that is useful breadth evidence, not arbitrary-file proof.

## Open gates

M1 is not complete. Still open are full Symusic scope/breadth (including score
operations and writer), finite Python-default limits, explicit EOT/division
strict-versus-compatible policy, fuzzing and arbitrary-corpus universality,
ARM64/portability evidence, isolated governor-controlled repeats, tiny native
2x performance, and a defensible theoretical-floor model.

The score v1 comparison is intentionally narrower than Symusic's product
surface: raw/lossless representation, writer, edits, time conversion, piano
rolls, ABC, synthesis, and SMPTE division are outside the equal-work claim.

Validation for this revision covered stable and Rust 1.97.1 workspace tests,
clippy, and no-default-feature checks; 85 Python tests passed with one skip;
the opt-in corpus differential passed its four checks; both native Rust/C++
verify-only paths passed; and the exact clean source commit was checked.

## Audited artifacts

- [preflight.json](../results/native-symusic-n5-final/preflight.json)
- [miso-a.json](../results/native-symusic-n5-final/miso-a.json) and
  [miso-b.json](../results/native-symusic-n5-final/miso-b.json)
- [symusic-a.json](../results/native-symusic-n5-final/symusic-a.json)
  and [symusic-b.json](../results/native-symusic-n5-final/symusic-b.json)
- [comparison.json](../results/native-symusic-n5-final/comparison.json)
- [score-n5-final.json](../results/score-n5-final.json)
- [retained-score-memory-n5-tiny-final.json](../results/retained-score-memory-n5-tiny-final.json)
  and [retained-score-memory-n5-final.json](../results/retained-score-memory-n5-final.json)
- [native-score-n5-final.json](../results/native-score-n5-final.json)

The historical N3 ablation evidence is also tracked so the factors above can
be recomputed from the matching native-comparison methodology rather than from
the unrelated native-score floor report:

- [N3 preflight.json](../results/native-symusic-n3-final/preflight.json)
- [N3 miso-a.json](../results/native-symusic-n3-final/miso-a.json)
  and [miso-b.json](../results/native-symusic-n3-final/miso-b.json)
- [N3 symusic-a.json](../results/native-symusic-n3-final/symusic-a.json)
  and [symusic-b.json](../results/native-symusic-n3-final/symusic-b.json)
- [N3 comparison.json](../results/native-symusic-n3-final/comparison.json)

See [the native comparison method](native-comparison.md), [the
benchmark contract](benchmark-contract.md), and [the retained-memory
method](retained-score-memory.md) for reproduction boundaries.
