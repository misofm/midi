# Native score parser and diagnostic floors

`benchmarks/native-score` is a dependency-minimal Rust workspace binary for
single-thread, warm in-memory measurements of
`miso_midi_core::parse_score_smf`. It is intentionally separate from the core
and binding packages; production code gains no benchmark dependency.

Run it from the repository root, ideally under an externally chosen CPU
affinity and with a clean release build:

```bash
mkdir -p benchmarks/results
taskset -c 4 cargo run -p miso-midi-native-score-bench --release -- \
  --datasets tiny,normal,huge,mahler \
  --output benchmarks/results/native-score-local.json
```

The defaults collect 30 samples after five warm-up operations. `--iterations 0`
(the default) calibrates a separate loop count for each product/floor operation
until one sample reaches 50 ms; set a positive `--iterations` to pin that
parameter. `--samples`, `--warmup`, `--min-sample-ns`, `--corpus-dir`, and the
comma-separated `--datasets` list are all recorded in the JSON.

For every requested corpus file, the harness:

1. loads bytes once and validates a built-in SHA-256 digest against the fixed
   expectation for the selected reproducible corpus;
2. parses a reference `TickScore` and validates its fixed semantic
   cardinalities, then records cardinalities and `heap_bytes` outside timing;
3. times `parse_score_smf(bytes)` with the produced score consumed through
   `black_box`; score destruction is part of that timed operation;
4. reparses and verifies the fixed hash/cardinality expectation after timing;
5. reports raw `ns/op` samples plus median/mean/min/max and median
ns/byte, ns/semantic-event, and ns/note.

For each diagnostic floor, the JSON also emits
`parse_median_to_floor_median_ratio`: the parse median divided by that floor's
median. This is a clearly named derived comparison only. It is not a percentage
of a theoretical limit, and the probes' optimistic assumptions mean it is not a
release efficiency claim.

`--parse-only` (or `--no-floors`) suppresses those diagnostic floor probes for
an equal native competitor comparison. It keeps the timed operation to
`parse_score_smf(bytes)` plus score consumption/destruction—no `note_count()`
or `heap_bytes()` traversal occurs inside that operation. `--verify-only`
performs the fixed input/full semantic-digest/cardinality check without timing.

The report also contains three **diagnostic floors** measured with the same
distribution machinery:

- `input_byte_touch`: one compiler barrier around the input slice, followed by
  a dependency-preserving checksum over every input byte;
- `output_allocation_request`: one contiguous allocation request equal to the
  parsed `TickScore` column-capacity bytes, without touching pages;
- `output_column_allocate_and_write`: that allocation plus one write to every
  byte.

These are deliberately optimistic calibrated work probes, not theoretical
limits. The parser has
variable-length decoding, semantic state, multiple allocations, sorting, and
metadata behavior that no one floor captures. Use them to identify whether an
input-read or output-allocation/write bound is plausible, then confirm with
isolated machines, `perf stat`, allocation profiling, and disassembly before
making a performance claim.

The machine metadata records target OS/architecture, `rustc --version`, the
Cargo profile injected by this package's build script, and the configured
workspace `[profile.release]` identity (`lto = "thin"`, `codegen-units = 1`,
`panic = "abort"`). Those fields describe the configured workspace profile;
they are not runtime proof of compiler flags. It also records debug assertions,
Linux process CPU-affinity input where available, CPU model, the selected first
affinity CPU's governor, and kernel release. The harness does not alter
affinity or governor; that is a lab-runner responsibility. Corpus files are locally generated or
checksum-pinned and ignored by default, so a published result must retain the
raw JSON and its hashes.

The current retained native raw artifact is
[native-score-n6-performance-final.json](../results/native-score-n6-performance-final.json).
Its medians, diagnostic floor ratios, and limits are summarized in the
[N6 safe-default evidence report](native-n6-evidence-2026-08-19.md). They are
specific to its non-isolated x86-64 environment; the diagnostic floors are not
a theoretical-limit or cross-library proof.

For the source-pinned C++ Symusic counterpart and the fail-closed native
comparison merger, see [native comparison](native-comparison.md).
