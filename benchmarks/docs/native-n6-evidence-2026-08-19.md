# N6 safe-default score evidence — 2026-08-19

N6 updates the score SDK to finite defaults and records the corresponding
evidence. It supersedes neither the [N5 native report](native-n5-evidence-2026-08-19.md)
nor its raw artifacts: N5 remains the historical unlimited-path baseline. This
report is scoped to the current score-v1 surface, one x86-64 host, and the
checked corpora. It is not a full Symusic-scope, hostile-input universality, or
theoretical-limit claim.

## Safety and equal work

The Python headline is `parse_score(data, limits=None, mode="compatible")`:
`None` selects finite core defaults, not unlimited parsing. The defaults are
64 MiB input, 4,096 source tracks, 16 MiB per source track, 2,000,000 events,
1,000,000 note starts, and 16 MiB normalized text. `parse_score_unlimited` is
an explicit trusted-only diagnostic escape hatch; it is unsafe for hostile
bytes and is not the Python headline.

Every Python dataset was preflighted under the complete
`miso-score-contract/v1` digest/count contract before timing. It covers TPQ,
ordered tracks/metadata, notes, controls, bends, pedals, lyrics, and global
time/key signatures, tempos, and markers. The generated files use valid matched
CC64 pairs. The 27-file, 1,717,487-byte upstream exact-Symusic-v0.6.0 corpus
also has full canonical equality evidence.

The native comparison is intentionally a different scope: it times the trusted
Rust `parse_score_smf` path against `symusic::Score<Tick>::parse<MIDI>(span)`
from exact Symusic v0.6.0 commit
[`43ff25277abbc72dbd8d00fb5a9a14ec37fb7906`](https://github.com/Yikai-Liao/symusic/tree/43ff25277abbc72dbd8d00fb5a9a14ec37fb7906),
built Release+IPO. It validates the same full contract outside timing and
includes score destruction inside timing. Therefore native numbers do not
measure finite-default policy cost.

## Finite-default Python result

This pyperf run used warm in-memory bytes, 20 processes × 3 values, CPU 4, and
a non-isolated `powersave` host. Smaller is better; speedup is Symusic/Miso
median.

| Dataset | Checked-default Miso | Symusic | Speedup |
| --- | ---: | ---: | ---: |
| tiny | 1.188533 µs | 2.558970 µs | 2.153x |
| normal | 134.056 µs | 372.456 µs | 2.778x |
| huge | 1.556315 ms | 4.344152 ms | 2.791x |
| Mahler | 806.723 µs | 1.777768 ms | 2.204x |
| geometric mean | — | — | **2.463x** |

The optional trusted diagnostic had medians of 1.123481 µs, 115.009 µs,
1.328947 ms, and 678.040 µs respectively. Checked-default/trusted median
ratios were 1.058x, 1.166x, 1.171x, and 1.190x. This isolates the current
finite-policy overhead only for these bytes and host; it is neither a
cross-library headline nor a claim that the trusted path is safe.

pyperf flagged Symusic huge and Miso-unlimited/Symusic Mahler under its <1%
stability criterion. Use raw distributions and medians, not a best sample or a
new mean headline.

## Native ABBA result and drift gate

The accepted native run temporarily set CPU 4 to `performance` and restored the
prior governor afterwards. It remains non-isolated. Each side has two raw
30-sample runs, pooled only after exact contract/provenance/build/host equality.
N6 additionally rejects every implementation/dataset whose two raw-run
medians differ by more than 5%; accepted Miso drift is at most 1.17% and
accepted Symusic drift is at most 0.46%.

| Dataset | Miso trusted native | Symusic native | Speedup |
| --- | ---: | ---: | ---: |
| tiny | 911.245 ns | 1,472.730 ns | 1.616x |
| normal | 114.235 µs | 351.596 µs | 3.078x |
| huge | 1.308402 ms | 4.122206 ms | 3.151x |
| Mahler | 674.853 µs | 1.654227 ms | 2.451x |
| geometric mean | — | — | **2.490x** |

The aggregate native 2x gate passes on this corpus/host. Tiny still fails a
per-dataset 2x gate. A separate `powersave` N6 candidate showed roughly 2x
Miso A/B drift and is deliberately rejected, ignored, and not an artifact for
this report.

## Retained RSS and floor diagnostics

Fresh Linux subprocess retained-score RSS slopes are equal-Python-API process
proxies: they include allocator behavior, native/proxy allocations, and handle
list overhead, not just native heap bytes.

| Dataset | Miso B/score | Symusic B/score | Miso/Symusic | ≤50% gate |
| --- | ---: | ---: | ---: | --- |
| tiny | 1,197.47 | 4,259.03 | 28.12% | pass |
| normal | 166,104.50 | 395,288.47 | 42.02% | pass |
| huge | 2,020,021.97 | 4,081,904.96 | 49.49% | pass |
| Mahler | 991,121.16 | 2,269,317.64 | 43.68% | pass |

Huge remains only 0.51 percentage points under the gate. Repeat it across
allocators and machines before treating that margin as durable.

The direct native parser/floor harness is diagnostic only:

| Dataset | Parse median | ns/byte | ns/event | byte-touch ratio | alloc+write ratio |
| --- | ---: | ---: | ---: | ---: | ---: |
| tiny | 903.701 ns | 5.285 | 41.077 | 72.80x | 74.24x |
| normal | 113.934 µs | 1.085 | 6.957 | 12.00x | 53.49x |
| huge | 1.309230 ms | 1.042 | 6.673 | 11.51x | 41.47x |
| Mahler | 670.932 µs | 1.022 | 6.905 | 11.31x | 39.71x |

Byte touch and one-contiguous allocation/write probes omit variable-length
decoding, score state, and real allocation behavior. Their ratios are not a
percentage of theoretical performance.

## Open boundary

Score-v1 excludes raw/lossless storage, writer, edits, time and SMPTE
conversion, piano rolls, ABC, and synthesis. Fuzzing/arbitrary-file
universality, full Symusic breadth, JavaScript SDKs, ARM64/portable evidence,
and isolated governor-controlled repeats remain open. The finite-default Python
API improves admission behavior, but is not a completed hostile-input campaign.

## Audited artifacts

- [score-n6-safe-default-final.json](../results/score-n6-safe-default-final.json)
- [preflight.json](../results/native-symusic-n6-performance-final/preflight.json)
- [miso-a.json](../results/native-symusic-n6-performance-final/miso-a.json),
  [miso-b.json](../results/native-symusic-n6-performance-final/miso-b.json),
  [symusic-a.json](../results/native-symusic-n6-performance-final/symusic-a.json), and
  [symusic-b.json](../results/native-symusic-n6-performance-final/symusic-b.json)
- [comparison.json](../results/native-symusic-n6-performance-final/comparison.json)
- [retained-score-memory-n6-tiny-final.json](../results/retained-score-memory-n6-tiny-final.json)
  and [retained-score-memory-n6-final.json](../results/retained-score-memory-n6-final.json)
- [native-score-n6-performance-final.json](../results/native-score-n6-performance-final.json)

See the [benchmark contract](benchmark-contract.md), [native comparison
method](native-comparison.md), and [resource policy](score-parser-resource-policy.md)
for the enforced boundaries.
