# Research and implementation roadmap

Date: 2026-08-19
Status: active program — M0 contract, N6 finite-default SDK policy, and x86-64 evidence implemented;
M1 score-parser vertical slice passes scoped native aggregate, Python, and RSS
gates, while breadth, portability, and release gates remain open
Competitive reference: Symusic 0.6.0 at commit
[`43ff252`](https://github.com/Yikai-Liao/symusic/tree/43ff25277abbc72dbd8d00fb5a9a14ec37fb7906)

## Executive decision

Miso MIDI should aim to become a portable symbolic-music core, not a faster
clone of either Mido or Symusic.

The first major product gate is nevertheless concrete:

1. Match Symusic 0.6's useful symbolic-MIDI scope.
2. Beat it decisively on equal-semantics native and Python benchmarks.
3. Move each hot kernel toward a measured hardware or materialization floor.
4. Freeze the core contracts only after those first three conditions hold.
5. Build Python, Node, browser/Wasm, and optional C SDKs as thin views over the
   same Rust implementation. Miso Engine consumes Rust directly.

This is a better north star than one headline parse number. Symusic's published
benchmark compares libraries with different output models and times path-based
loads. Miso's claims must state the input, validation, semantic output, cache
state, memory behavior, and SDK materialization policy.

The working performance objective is:

> For each operation, do only the semantic work requested, make the minimum
> practical number of passes and allocations, and approach the calibrated
> input-read, output-write, compute, or FFI floor on both x86-64 and ARM64.

"Theoretical limit" is not one number. Parsing, transposition, piano-roll
construction, serialization, and Python object creation have different
unavoidable work. The benchmark lab defined below gives each kernel its own
floor.

### Starting point

The repository has a safe dependency-free scanner, a compact event arena, a
PyO3 wrapper, deterministic valid-pedal corpora, malformed cases, Rust checked
score-parser limits, and the first native-backed `parse_score` vertical slice.
M0's canonical score contract now checks TPQ, ordered tracks, notes, controls,
pitch bends, pedals, text, and global metadata before any cross-library score
timing or retained-memory worker starts. An opt-in, checksum-pinned 27-file
Symusic testcase differential adds real-world breadth evidence without
vendoring MIDI bytes.

The current x86-64 result is documented in [the N6 safe-default evidence
report](native-n6-evidence-2026-08-19.md). Against exact source-pinned Symusic
0.6.0 and full-contract preflight, the accepted 60-sample/side native ABBA run
reaches a 2.490x median-geomean advantage after a 5% A/B median-drift gate;
tiny is 1.616x and misses a per-dataset 2x target. The finite-default
Compatible Python median geometric mean is 2.463x, with trusted-path policy
overhead reported separately. The scoped retained-RSS gate passes; huge is
49.49% of Symusic and narrow. RSS remains an equal-Python-API process proxy,
not a pure native allocator measurement. No ARM64 audit, arbitrary-universe
corpus parity result, or full Symusic scope exists. Writers/operations and fuzz
coverage remain open; M0 multi-architecture and M1 release gates are incomplete.

## Scope to match

Symusic 0.6 is a note-level toolkit backed by C++20 and nanobind. Its current
[feature list](https://github.com/Yikai-Liao/symusic) and bindings establish the
following comparison surface.

| Area | Symusic 0.6 surface | Miso target |
| --- | --- | --- |
| MIDI I/O | Parse bytes/path; dump MIDI | Strict, compatible, lossless, and normalized SMF read/write |
| Score model | Score, tracks, notes, pedals, CC, pitch bend, lyrics, markers, tempo, key/time signatures | Equivalent musical information in compact arenas, plus lossless raw events |
| Time | Tick, quarter, second; tempo-map conversion; resampling | Typed domains with checked conversion and reusable tempo indices |
| Queries | Start/end, note counts, empty, beats, downbeats | Same, with cached or single-pass bulk kernels |
| Editing | Sort, filter, clip, trim, adjust time, shift time/pitch/velocity | Same semantics plus fused edit pipelines |
| Bulk data | Per-event and list NumPy conversion | Zero/one-copy column views and bulk import/export |
| Piano roll | Track/score dense arrays, modes, velocity encoding | Dense and sparse outputs with explicit size limits |
| Persistence | Fast pickle through zpp_bits | Stable, versioned native format plus Python pickle hooks |
| ABC | `abc2midi`/`midi2abc` subprocess adapters | Feature-gated adapter, not part of the trusted core |
| Synthesis | Feature-facing SoundFont rendering through prestosynth | Separate optional synth crate/adapter and benchmark domain |
| Distribution | CPython wheels across major desktop platforms | Rust crate, CPython, Node, and browser artifacts from one conformance suite |

Parity means matching the useful capability and documented semantics, not
copying Symusic's Python names, memory model, serialization bytes, or bugs.
ABC and synthesis count toward breadth but must not add weight, unsafe code, or
latency to the default MIDI core.

Out of scope until the parity/performance gate passes:

- a Mido drop-in API;
- music notation/layout, MusicXML, audio analysis, or DAW project formats;
- live MIDI and MIDI 2.0 UMP beyond preserving the architecture hooks already
  described in `architecture.md`;
- transparent mutation of millions of individual Python or JavaScript objects;
- parallel speedups used to disguise weak single-core kernels.

## Product architecture

One decoder should feed two first-class representations and build neither when
the caller does not request it.

```text
bytes / mmap / SDK buffer
          |
          v
   SMF framing + decoder
      /             \
     v               v
lossless EventArena  fused ScoreBuilder -> columnar ScoreArena<T>
     |                         |
raw views / writer       queries / edits / time conversion
      \                       /
       +--- bulk columns ----+
                  |
        Rust / Python / Node / Wasm / C
```

### Event layer

Keep the existing compact `OwnedSmf` trajectory and add borrowed traversal and
writing:

- `EventIter<'a>` for allocation-free scan/decode;
- `EventArena` for track ranges, fixed event headers, and payload blobs;
- lossless preservation of unknown chunks, unknown meta events, SysEx, and
  encoding details needed for fidelity-oriented output;
- normalized views for callers that only want semantic MIDI messages.

### Score layer

Add `ScoreArena<T>` as a hybrid structure-of-arrays representation:

- global note columns for start, duration, pitch, and velocity;
- track ranges and compact track metadata rather than one allocation per track
  or event;
- separate columns for CC, pedals, pitch bends, tempo, signatures, markers,
  and lyrics;
- interned or blob-backed UTF-8 text with offset/length pairs;
- checked integer ticks and `f64` quarter/second domains;
- builders for mutation and immutable snapshots/views for cheap sharing;
- optional cached sortedness, extents, and tempo-map indices.

The score fast path must decode directly into `ScoreBuilder`; it must not first
materialize every wire event. A caller requesting both representations may
share decoding, but the common `from_midi -> Score` case gets a fused pass.

Start with modules inside `miso-midi-core`. Extract `miso-midi-smf`,
`miso-midi-score`, `miso-midi-serialize`, `miso-midi-abc`, or
`miso-midi-synth` only when their feature, allocation, or versioning contracts
actually diverge. Premature crate boundaries make profile-guided changes and
API iteration harder.

### Note construction

Use bounded direct indexing for channel/pitch state:

- primary state is `[channel][pitch]`, never a general hash lookup;
- a small spill structure handles overlapping notes at the same channel and
  pitch;
- FIFO/LIFO/all-stop policies are explicit and tested rather than accidental;
- track grouping by source track, channel, program, and drum role has a written
  compatibility policy;
- orphan note-offs and unterminated notes have strict and compatible outcomes;
- controls and text seen before the first note do not force a heavyweight
  track allocation.

Index every track chunk first. Type-1 tracks can later be decoded in parallel,
but single-thread latency is the primary gate and parallelism has a measured
size threshold.

### Mutation model

Do not let SDK object ergonomics dictate core layout.

- `ScoreArena` is an immutable, shareable snapshot.
- `ScoreBuilder` or `EditSession` owns mutable columns and validates on commit.
- simple whole-column operations can mutate uniquely owned storage in place.
- non-inplace operations use copy-on-write where a measured win justifies it.
- SDK note objects are index-backed views or detached values, with explicit
  lifetime and invalidation rules.
- bulk edit plans fuse compatible operations, such as clip + transpose +
  velocity clamp, into one pass.

## Defining "close to the limit"

### Calibrated floors

The benchmark machine is characterized at the start of every serious run.
Floors are measured with the same compiler, allocator, CPU affinity, and buffer
sizes as the product kernel.

| Kernel | Unavoidable-work floor |
| --- | --- |
| Structural scan | Touch/checksum every input byte plus minimal chunk framing |
| Event/score parse | Input read plus allocation and writes for the exact requested arena |
| In-place transform | Read and write only the affected columns |
| Copying transform | Allocate and copy unchanged output columns plus transform changed columns |
| Sort | Already-sorted detection floor; key extraction plus comparison/radix floor otherwise |
| Time conversion | Read source time columns, tempo lookup, and write destination columns |
| Serialization | Read arena and write the exact encoded byte count |
| Bulk SDK export | Constant-time shared view where legal; otherwise exact byte-copy floor |
| Python objects | Native call plus construction of the same number and shape of Python objects |
| Dense piano roll | Allocate/clear every output cell plus write the requested note spans |
| Synthesis | Required voices, sample interpolation, mixing, and exact output frames |

For each kernel record:

```text
efficiency ratio = observed kernel time / calibrated unavoidable-work floor
```

The floor is a diagnostic bound, not a marketing result. Publish the floor
microkernel and its assumptions next to the product benchmark. Use multiple
floors when the lower bound is ambiguous: byte touch, output allocation/write,
and a minimal semantic "shadow kernel."

### Provisional performance gates

M0 research should adjust the numeric thresholds if the data shows they are
physically inconsistent. Until then, a release candidate must meet all of the
following on the primary x86-64 and ARM64 machines:

1. Native tick-score parse is at least **2.0x faster** than Symusic's geometric
   mean over the reference corpus at equal semantics.
2. Python `Score.from_midi(bytes)` is at least **1.75x faster** at equal lazy or
   materialized output policy.
3. Primary transforms, time conversion, MIDI dump, native serialization, and
   piano-roll cases are at least **1.5x faster**, unless Miso is already within
   **1.25x of the calibrated floor** for that output.
4. No representative corpus decile or benchmark cell is more than **10% slower**
   without a documented correctness or safety reason.
5. Persistent native score memory is at most **50% of Symusic's** for the same
   information; peak memory is reported separately.
6. A native-backed SDK operation adds less than **5%** over its Rust kernel for
   medium and large inputs. Object materialization is separately labeled.
7. Every optimized kernel preserves the scalar reference implementation's
   result and malformed-input behavior.

Use single-thread results for these gates. Report opt-in batch throughput and
parallel scaling separately.

### Metrics

Wall time alone is insufficient. Capture:

- median, distribution, confidence interval, and raw samples;
- bytes/s, events/s, and notes/s;
- cycles/byte, cycles/event, instructions/event, IPC;
- branches and branch misses, L1/LLC misses, context switches;
- allocation count, allocated bytes, peak RSS, and retained bytes;
- input size, semantic output count, and output byte size;
- CPU model/microcode, frequency policy, memory, OS, compiler flags, allocator,
  Rust/Python/library versions, and corpus hashes.

Profile before using SIMD or unsafe code. SMF's variable-length and branchy
grammar may favor scalar state machines; stable Rust portable SIMD is still
experimental as of Rust 1.97. Use target-specific intrinsics only behind a
scalar fallback and only after a benchmark proves an important win.

## Benchmark and correctness laboratory

### Workload layers

Never collapse these into one "parse" result:

1. warm in-memory structural scan;
2. warm in-memory lossless event arena;
3. warm in-memory normalized tick score;
4. quarter and second score construction/conversion;
5. cold path load and warm page-cache load;
6. normalized and fidelity-oriented MIDI dump;
7. each query/edit operation, isolated and fused;
8. native serialization/deserialization and Python pickle;
9. dense/sparse piano-roll construction;
10. bulk NumPy and typed-array views/copies;
11. detached Python/JavaScript object materialization;
12. malformed rejection and resource-limit enforcement;
13. synthesis, in a separate audio benchmark suite.

Every cross-library case computes a canonical semantic digest outside the timed
region and rejects unequal results. Track input parsing separately from file
I/O. Keep Symusic pinned by version and wheel hash.

### Corpus matrix

Retain the generated local tiny, normal, huge, malformed, and checksum-pinned
Mahler cases. Add deterministic factor generators for:

- SMF format 0/1/2, track count, event density, and empty tracks;
- one-to-four-byte VLQs and near-overflow absolute times;
- running-status frequency and status-change frequency;
- note/controller/meta/SysEx/text mixes;
- tempo and signature density;
- repeated and overlapping same-pitch notes;
- sorted, reverse-sorted, and slightly disordered scores;
- long notes, zero-duration notes, pedals, orphan/dangling notes;
- large text/SysEx payloads and adversarial chunk sizes;
- SMPTE division and unknown chunks/events.

Add a stratified, license-audited local corpus drawn from the same broad shapes
used by the current
[Symusic benchmark](https://github.com/Yikai-Liao/symusic-benchmark): piano,
pop, ensemble, very small files, and very large files. Dataset validation must
not use Symusic itself as the only admission oracle. Check in manifests,
hashes, generators, and redistributable fixtures; keep restricted datasets as
optional downloads.

### Measurement tiers

- **PR smoke:** deterministic correctness plus short regression checks; never
  make absolute speed claims from shared CI runners.
- **Nightly lab:** isolated CPU, fixed/published frequency policy, Criterion or
  equivalent native distributions, pyperf process isolation, and `perf stat`.
- **Release audit:** x86-64 and ARM64 lab machines, clean builds, cold/warm I/O,
  memory profiles, full corpus, raw JSON, and reproducible report generation.
- **Instruction audit:** Iai/Callgrind-style deterministic instruction and cache
  counts for small kernels, plus disassembly/LLVM-MCA inspection where useful.

Pyperf supports worker processes, warmups, affinity, JSON, and memory tracking;
use those facilities instead of minimum-of-`timeit` results. Native benchmarks
must provide profiling modes and saved baselines.

### Correctness and security gates

- A slow, obvious Rust reference decoder and score builder are semantic oracles.
- Differential tests compare raw semantics with at least two independent MIDI
  implementations and score semantics with Symusic plus another note-level
  implementation.
- Golden vectors define note pairing, program changes, channel 10 drums, text
  decoding, tempo defaults, event ordering, and conversion rounding.
- Property tests cover encode/decode and transform invariants.
- Coverage-guided fuzzers target scan, parse, score build, write, deserialize,
  time conversion, and piano roll; every finding becomes a fixture.
- Malformed input must terminate in bounded time with stable error kind and
  byte offset, no panic, and no allocation before checked limits.
- Explicit limits cover input bytes, chunks, tracks, events, absolute time,
  text/SysEx bytes, output cells, decoded duration, and serialized allocation.
- Miri, sanitizers where applicable, and dependency audits run before releases.
- `unsafe` remains forbidden until a reviewed benchmark demonstrates a
  material gain unavailable in safe Rust. Each exception gets invariants,
  fuzz coverage, sanitizer coverage, and a scalar safe fallback.

## Research program

Each track ends in an ADR, a minimal prototype, raw benchmark data, correctness
vectors, and a keep/change/reject decision. Research does not merge as an
unmeasured permanent abstraction.

### R0: Freeze the comparison contract

- Turn the Symusic API/test inventory into a machine-readable parity matrix.
- Pin Symusic 0.6.0, Python, compiler, and benchmark corpora.
- Specify canonical semantic records for event, tick-score, converted-score,
  piano-roll, and MIDI-output comparisons.
- Document strict/compatible behavior for every SMF edge case.

Exit: the harness fails visibly when two libraries perform unequal work.

### R1: Characterize the machines and floors

- Add byte-touch, memcpy, allocation, column-copy, FFI-call, Python-object,
  typed-array, varint, and branch-dispatch microbenchmarks.
- Capture counters and make roofline/floor reports reproducible.
- Measure the existing scanner and event arena against those floors.

Exit: every headline benchmark can name its dominant lower bound and efficiency
ratio.

Status on 2026-08-19: the dependency-minimal native `parse_score_smf` harness
emits warm-byte distributions, full semantic contracts, corpus hashes, and
byte-touch/output allocation-write diagnostic probes. Its N5 substantive
inputs measure 1.03–1.12 ns/byte and 6.93–7.20 ns/event; the 11.35–12.43x
byte-touch ratios are diagnostics, not theoretical efficiencies. Isolated
x86-64 and ARM64 audits, counters, and allocation profiles remain required.

N1/N6 now has a source-pinned C++ Symusic harness and fail-closed merger using
the public `Score<Tick>::parse<MIDI>(span)` API, exact v0.6.0 provenance,
independent native digests, ABBA pooled reports, and a 5% A/B median-drift
gate. The N6 native aggregate is 2.490x, but tiny misses 2x; see the
[N6 report](native-n6-evidence-2026-08-19.md).

### R2: Score representation bake-off

Prototype on the same corpus:

- packed array-of-structs;
- global SoA with track ranges;
- per-track SoA/chunked columns;
- immutable `Arc` snapshots versus unique mutable builders;
- 32-bit versus 64-bit offsets and checked large-file mode;
- borrowed versus copied text payloads.

Evaluate parse/write speed, common transforms, iteration, random note access,
bulk export, retained memory, and SDK handle cost. Select one default and keep
conversion paths only where measured use cases require them.

Exit: `ScoreArena<T>` layout and ownership ADR accepted.

Status on 2026-08-19: the prototype provisionally selected safe transparent
little-endian rows: `[u8; 10]` controls plus adaptive note rows (`[u8; 10]`
narrow and `[u8; 18]` wide), with lazy legacy-note materialization. Four-column
SoA and direct two-column layouts were slower; byte-packed rows recovered
speed; representation-packed rows had no material x86 win and were rejected
for portability; hot/cold state separation supplied the remaining win. This is
an evidence-backed vertical-slice decision, not the final breadth/layout ADR.

### R3: Fused score decoder

- Decode SMF chunks directly into score columns.
- Implement direct-index note matching and explicit overlap policy.
- Avoid maps in the common channel/program path; measure alternatives for
  program-split track lookup.
- Pre-count only when a second pass beats amortized growth.
- Test fast ASCII/UTF-8 paths without losing validation semantics.
- Explore track-level parallel parsing only after the single-core gate.

Exit: full tick-score semantic parity and the provisional 2x native parse gate.

### R4: Writer and fidelity modes

- Implement exact delta/VLQ sizing, running-status selection, stable event
  ordering, and caller-provided output buffers.
- Separate normalized score output from lossless event replay.
- Avoid a general sort when sortedness is known; benchmark merge-based assembly
  and specialized key/radix strategies.

Exit: deterministic round-trip suites pass and MIDI dump meets its gate.

### R5: Time and musical queries

- Build a compact, reusable tempo segment index.
- Define exact rounding, overflow, sentinel defaults, and resampling policies.
- Implement tick/quarter/second conversion, resample, start/end/count, beats,
  and downbeats.
- Benchmark binary search, galloping cursors, and monotonic one-pass conversion.

Exit: parity across tempo/signature adversaries with no unchecked arithmetic.

### R6: Editing kernel family

- Implement sort/is-sorted, filter, clip, trim, shift pitch/velocity/time, and
  piecewise `adjust_time` first as scalar references.
- Add column-specialized in-place and copying kernels.
- Introduce fused edit plans only after isolated kernels are correct.
- Measure stable versus unstable ordering and radix versus comparison sorting.

Exit: operation parity and either the competitor or floor gate for every
primary kernel.

### R7: Bulk interop and native persistence

- Expose typed, lifetime-safe column descriptors.
- Prototype zero-copy NumPy views, Arrow-compatible buffers only if demanded,
  Node typed arrays, and Wasm linear-memory views.
- Evaluate native formats on decode speed, mmap/view feasibility, schema
  evolution, validation cost, portability, and untrusted-input safety.
- Version the selected format and add migration/golden-file tests.

Exit: bulk exports are zero-copy where ownership permits, and serialization
meets a written compatibility policy.

### R8: Piano rolls

- Specify modes, pitch ranges, velocity aggregation, overlaps, and bounds.
- Provide dense and sparse outputs; require callers to opt into enormous dense
  allocations.
- Compare span filling, difference arrays, tiling, and parallel track planes.

Exit: bit-identical/array-identical parity cases, checked dimensions, and a
floor-relative performance result.

### R9: Optional breadth

- ABC: first match Symusic through a sandboxable, explicit subprocess adapter;
  separately evaluate a native parser only if product demand justifies it.
- Synthesis: define SoundFont compatibility and audio-quality fixtures, then
  compare an existing Rust backend, a C adapter, and Miso Engine reuse. Keep
  rendering out of the base crate and parser benchmark.

Exit: installable optional features with isolated licenses, dependencies,
security boundaries, and benchmarks.

### R10: SDK ergonomics and cost

- Validate coarse FFI calls, index-backed event/note views, bulk columns,
  mutation sessions, async/batch parsing, errors, and object detachment.
- Measure the cost of every abstraction against a Rust call.
- Use one language-neutral conformance-vector format across SDKs.

Exit: core ownership and error contracts can support Python, Node, Wasm, and C
without language-specific state inside the core.

## Implementation milestones and gates

Milestones are capability gates, not dates. Do not start a later SDK breadth
phase merely because time elapsed.

### M0: Benchmark truth

Deliver:

- parity matrix and semantic digests;
- pinned Symusic runner and full benchmark matrix;
- native/SDK microfloors and hardware reports;
- expanded corpus, malformed fixtures, and initial fuzz targets.

Gate: repeatable equal-work numbers on x86-64 and ARM64.

Status on 2026-08-19: the pinned Symusic 0.6.0 runner, canonical digest/count
preflight, valid generated pedal fixtures, raw pyperf JSON, Linux retained-RSS
harness, native parser/floor JSON, and a native ABBA result exist. N6's
aggregate native 2x and scoped Python/RSS gates pass on non-isolated x86-64;
tiny misses native 2x and floor ratios remain diagnostic. Rust checked
score-parser limits and a 27-file/1,717,487-byte opt-in testcase differential
exist. ARM64, broader parity/fuzz coverage, finite-default adversarial testing,
and release-quality machine controls remain.

### M1: Tick score vertical slice

Deliver:

- selected `ScoreArena<Tick>` layout;
- fused score parse with notes, tracks, CC, pitch bend, pedals, tempos,
  signatures, lyrics, and markers;
- strict/compatible limits and Python score summary/bulk access;
- differential and property tests.

Gate: tick-score parity, memory target, and 2x native/1.75x Python parse target.

Status on 2026-08-19: the Python-exposed tick-score prototype constructs the
contract fields for the generated/checksum-pinned local corpus and is
differentially preflighted against Symusic. N6 meets the aggregate native 2x,
Python 1.75x, and scoped equal-API retained-RSS gates on one x86-64 host;
native tiny remains below 2x and huge RSS is narrow. Full arbitrary-file/parity
breadth, operations, time domains, writer, bulk views, ARM64, and isolated
repeat work remain pending. Rust checked resource limits are implemented and
the Python binding now defaults to finite Compatible parsing; the explicit
unlimited API is reserved for trusted input.

The opt-in, checksum-pinned [Symusic v0.6.0 testcase corpus](upstream-testcase-corpus.md)
adds 27 non-vendored one-track and multitrack real-world files with a full
canonical-contract differential preflight. It is breadth evidence only: it is
not timing data and does not establish arbitrary-file parity or fixture
redistribution rights.

### M2: Complete SMF round trip

Deliver:

- borrowed event iterator;
- normalized writer and lossless event writer;
- unknown event/chunk preservation;
- canonical output and fidelity fixtures.

Gate: stable deterministic round trips and writer performance target.

### M3: Score operations and time

Deliver:

- quarter/second conversion and resampling;
- tempo index, beats/downbeats;
- all primary query/edit operations and edit fusion.

Gate: Symusic operation breadth and per-kernel performance gates.

### M4: Analytical interop

Deliver:

- NumPy bulk import/export;
- dense/sparse piano rolls;
- native versioned serialization and Python pickle;
- memory mapping if R7 validates it.

Gate: no required per-event FFI loop, checked dimensions, persistence policy,
and interop performance gates.

### M5: Harden and declare core beta

Deliver:

- full fuzz/property/differential corpus;
- resource-limit and denial-of-service review;
- benchmark dashboard with regression budgets;
- public semantic, ownership, error, and stability contracts.

Gate: no known correctness blockers; full release audit passes on both lab
architectures; the core schema is stable enough for multiple SDKs.

### M6: Optional Symusic breadth

Deliver feature-gated ABC conversion and SoundFont synthesis with their own
quality, security, packaging, and performance reports.

Gate: installing the base core does not bring either feature's dependencies.

### M7: Multi-language SDKs

Deliver Python first as the reference SDK, then Node, browser/Wasm, and optional
C. Release each only when it passes the same golden vectors and overhead gates.

Gate: one semantic core, no duplicated parser or transform implementations.

### Critical path

```text
M0 benchmark truth
        |
        v
M1 tick-score vertical slice
       / \
      v   v
 M2 writer  M3 time + operations
       \   /
        v v
 M4 analytical interop
        |
        v
 M5 hardened core beta
       / \
      v   v
 M6 ABC/synth  M7 Node/Wasm/C SDKs
```

Python evolves alongside M0-M5 because it is both the reference SDK and the
competitive test surface. After M1, writer/fidelity and score-operation work can
proceed in parallel. Node/Wasm should wait for M5's ownership and schema freeze;
otherwise binding churn will consume the performance work.

## SDK sequence

### Python reference SDK

Python remains first because it is the direct Symusic comparison and the main
MIR adoption path.

- PyO3 + maturin, managed by `uv`;
- release the GIL around native kernels and audit free-threaded Python support;
- accept bytes, buffer-protocol inputs, paths, and batches;
- lightweight `Score`, `Track`, and event views backed by Rust ownership;
- optional NumPy dependency, structured/column exports, pickle, type stubs, and
  Pythonic detached constructors;
- distinguish `columns()`/views from `to_objects()` in names and benchmarks;
- publish abi3/abi3t strategy only after measuring compatibility and overhead.

### Rust and Miso Engine

Rust is not an FFI SDK. Expose validated arenas and transformations directly.
Miso Engine converts an offline score into its admitted, bounded scheduling
format before playback; parsing and allocation never enter the audio callback.

### Node and browser

- napi-rs Node binding with `Buffer`/typed-array bulk exchange;
- wasm-bindgen browser backend with explicit copy versus borrowed linear-memory
  views;
- one TypeScript facade with capability reporting, not hidden semantic
  differences;
- worker-friendly batch APIs and no event-per-call boundary.

Node external buffers can be zero-copy only where runtime ownership permits;
the SDK must expose and benchmark fallback copies rather than promise universal
zero-copy behavior.

### Optional C ABI

Add only after the Rust ownership/error contract stabilizes:

- versioned functions and opaque handles;
- explicit create/retain/release and borrowed-span lifetimes;
- numeric error codes with offset/context accessors;
- column descriptors and caller-owned output buffers;
- ABI tests from C and at least one non-C host.

The C ABI is an adapter, never the implementation boundary used by Miso Engine.

## Operating model

For each optimization:

1. Add or identify the correctness vector and benchmark cell.
2. Measure time, counters, allocation, memory, and current floor distance.
3. Profile and state the bottleneck hypothesis.
4. Implement the smallest representation or kernel change that tests it.
5. Re-run correctness, fuzz regressions, multi-architecture benchmarks, and
   code-size checks.
6. Keep the change only if the end-to-end win is material and the maintenance
   cost is documented.

Performance budgets live beside tests. A PR cannot trade correctness, bounded
failure, or portability for a benchmark win without an explicit design review.
Publish raw results and semantic checksums so outside contributors can reproduce
claims.

## Immediate implementation queue

The contract, N6 finite-default/native evidence, scoped Python/RSS gates, and native floor
distributions now exist. The next changes should close the open gaps rather
than repeat the same headline benchmark:

1. **Native/floor audit:** repeat the source-pinned native comparison on
   isolated x86-64 and ARM64 hosts, add counters/allocation profiles, and
   investigate the remaining tiny <2x and 11–12x byte-touch diagnostic gaps.
2. **Contract breadth and limits:** expand differential vectors beyond the
   generated/checksum-pinned local corpus,
   including edge ordering, text, overlap, program/channel grouping, malformed
   resource limits, and strict/compatible policies; define enforced resource
   bounds rather than merely testing malformed inputs.
3. **Layout hardening:** retain the selected safe adaptive-row design as a
   provisional ADR, add multi-machine retained-memory repeats, and keep
   portable alternatives only where measured workloads need them.
4. **M1 completion decision:** do not mark M1 complete until breadth, bounded
   Python defaults, strict/compatible policy, portability, and isolated-repeat
   gaps are resolved; writer, time conversion, editing, and bulk APIs may be
   prototyped behind that boundary.

## Primary source notes

- Current Symusic architecture and features:
  [repository README](https://github.com/Yikai-Liao/symusic) and
  [MIDI operations documentation](https://symusic.readthedocs.io/en/stable/tutorials/midi_operations.html).
- Current comparison harness:
  [symusic-benchmark](https://github.com/Yikai-Liao/symusic-benchmark), which
  uses path-based read/write timing and minimum-of-`timeit` samples.
- Python measurement controls:
  [pyperf runner](https://pyperf.readthedocs.io/en/latest/runner.html) and
  [system tuning](https://pyperf.readthedocs.io/en/latest/system.html).
- Python distribution tradeoffs:
  [PyO3 features](https://pyo3.rs/main/features).
- Node buffer ownership and zero-copy limitations:
  [napi-rs typed arrays](https://napi.rs/docs/concepts/typed-array).
- Rust portable SIMD status:
  [`std::simd`](https://doc.rust-lang.org/std/simd/index.html).
