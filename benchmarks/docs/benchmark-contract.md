# Benchmark contract

## Questions

The suite answers separate questions rather than collapsing them into one parse
time:

- How quickly can the core validate and identify every SMF event boundary?
- What does compact owned materialization cost?
- What does a high-level SDK object model cost?
- How much time is file I/O versus decoding?
- How do latency and memory scale with event count, tracks, SysEx, text, running
  status, and VLQ widths?
- Do malformed inputs fail in bounded time?

## Corpus

The generator creates deterministic local cases:

| Case | Shape | Purpose |
| --- | --- | --- |
| `tiny` | one short track | call and fixed-cost overhead |
| `normal` | eight medium tracks | ordinary multitrack work |
| `huge` | sixteen dense tracks | decoder throughput and allocation pressure |
| `mahler` | checksum-pinned public reference | reproduce the widely quoted Symusic table |
| `malformed-*` | truncated and invalid structures | bounded rejection and diagnostics |

The original 2.92-second Mido result used `mahler.mid` on an i7-10875H laptop.
It compared event-level and note-level libraries and used `timeit`. We retain the
file as a historical reference, not as the sole product benchmark.

## Workload labels

- `miso/scan`: crosses Python once and scans all event boundaries in Rust. It
  does not create per-event Python objects.
- `miso/parse-arena`: fully decodes into compact Rust-owned track ranges, event
  headers, and payload storage.
- `mido/parse-objects`: parses from an in-memory buffer and eagerly creates Mido
  track and message objects.
- `miso/semantic-records` and `mido/semantic-records`: parse and convert into
  identical `(track, delta, status, meta_type, payload)` Python tuples.
- `miso/parse-score` and `symusic/parse-score`: construct tick-score objects
  from the same warm in-memory bytes. Before either is timed, both are checked
  against the full `miso-score-contract/v1` canonical digest and output counts;
  digest generation is outside the timed region.
- `miso-unlimited/parse-score`: an opt-in diagnostic series emitted only with
  `bench_score.py --include-miso-unlimited`. It calls the explicitly trusted,
  unlimited Python parser and must preflight against the same full contract.
  It never replaces the finite-default `miso/parse-score` headline or changes
  interpretation of historical two-series JSON artifacts.
- Path-based cases will measure warm-cache file I/O separately from in-memory
  decoding.

The compact-representation comparison is a valid product-level latency and
memory comparison, but it reflects different object models. The semantic-record
comparison performs equal observable work and is the primary cross-library
speed claim. Inputs, validation mode, cache state, interpreter, CPU, and
measurement method must be published next to either result.

The score contract includes TPQ; ordered track metadata; notes, controls, pitch
bends, pedals, lyrics; and global signatures, tempos, and markers. Generated
fixtures use explicit matched sustain on/off pairs and exclude CC64 from their
arbitrary-controller stream. This prevents an implementation's behavior for an
unmatched pedal transition from changing the comparison work.

When the unlimited diagnostic is present, the score summarizer prints the
checked/default-over-unlimited median and mean ratios in a separate policy
overhead table. It fails closed if that third series is partial or has unequal
contract metadata. The primary Miso/Symusic table and geometric means remain
the finite-default values.

## Retained-score memory

`benchmarks/measure_score_memory.py` is intentionally a separate, non-timing
measurement. It preflights the complete score contract, then launches a fresh
Linux subprocess for each library/dataset pair. Each worker records current RSS
after selected-library import and input loading, retains scores through multiple
checkpoints in a preallocated Python list, and reports raw checkpoints, an RSS
slope, final inclusive bytes/score, and Python list/handle overhead.

It refuses unsupported platforms rather than substituting a different memory
metric. Current RSS is a process-level Linux signal, not a portable heap
counter; `heap_bytes` from one implementation cannot be compared directly with
it. See [the retained-memory method](retained-score-memory.md) and
the [N6 safe-default evidence report](native-n6-evidence-2026-08-19.md). N5,
the earlier [R2 score-layout report](score-layout-r2-2026-08-19.md), and the
[baseline report](python-score-baseline-2026-08-19.md) remain available for comparison.

## Native parser and floor probes

`benchmarks/native-score` times Rust `parse_score_smf` directly with warm
in-memory bytes and consumes every score through `black_box`; score destruction
is deliberately included in the timed operation. Fixed corpus SHA-256 values
and semantic cardinalities are checked before and after timing, then raw distributions report
ns/byte, ns/semantic-event, and ns/note. It records corpus SHA-256 values,
compiler/profile, and process affinity metadata.

The adjacent byte-touch and output allocation/write microkernels are diagnostic
and optimistic work probes, not theoretical-limit claims. Their assumptions and raw samples
are in the same JSON so a report can state exactly which floor was considered.
Each probe includes a clearly named parse-median/floor-median derived ratio; it
is not a percent-of-theoretical value. See [the native-score method](native-score-benchmark.md)
and its [N6 raw-result interpretation](native-n6-evidence-2026-08-19.md).

The native Rust-versus-Symusic comparison path is separately source-pinned and
requires the same full M0 contract preflight before it merges distributions;
see [native comparison](native-comparison.md). Both native
harnesses independently verify the full canonical digest before/after timing,
and the merger only pools ABBA reports with matching source, release/IPO,
configuration, CPU model/affinity/governor/kernel conditions, and a maximum 5%
per-implementation/dataset A/B raw-run median drift. The published N6 result
is [2.490x native median-geomean evidence](native-n6-evidence-2026-08-19.md) on
one non-isolated x86-64 host; tiny remains below 2x and broad parity remains open.


## Release gates

1. All valid corpus files agree with at least two independent implementations
   on event boundaries and header semantics.
2. All malformed fixtures and fuzz-discovered regressions have stable outcomes.
3. Benchmark JSON includes tool versions, platform metadata, corpus hashes, and
   raw samples.
4. Performance regressions are assessed by distribution, not one best run.
5. Peak memory and allocations are reported alongside latency for owned and SDK
   representations.
