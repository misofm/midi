# Miso MIDI versus Mido — 2026-08-19

This benchmark compares Miso MIDI's Rust-backed Python package with Mido 1.3.3.
Every case parses from an already-loaded `bytes` object, so file-system I/O is
excluded.

## Correctness contract

Both libraries are converted into ordered tuples with the shape:

```text
(track_index, delta_ticks, status, meta_type_or_none, payload_bytes)
```

All generated fixtures and the checksum-pinned `mahler.mid` produce identical
tuples. The Mahler comparison covers 33 tracks and 157,980 events. `SysEx`
framing is normalized to Mido's observable representation.

## Environment

- AMD Ryzen 7 9700X, 8 cores / 16 threads
- Linux 6.8.0-138, x86-64, glibc 2.39
- CPython 3.12.3
- Rust 1.97.1; release extension with thin LTO
- Mido 1.3.3, pyperf 2.10.0
- 2 worker processes, 3 values, 1 warmup, 50 ms minimum value time

## Results

### Native representation

| Corpus | Events | Miso compact arena | Mido object graph | Ratio |
| --- | ---: | ---: | ---: | ---: |
| tiny | 38 | 265 ns | 64.5 µs | 243× |
| normal | 32,376 | 170 µs | 57.5 ms | 338× |
| huge | 388,192 | 2.04 ms | 730 ms | 358× |
| mahler | 157,980 | 960 µs | 292 ms | 304× |

This is a legitimate product-level comparison—both results expose every parsed
event—but the representations differ. Miso stores 16-byte native event headers
and shared payload bytes; Mido creates one Python message object per event.

### Equal Python output

| Corpus | Events | Miso semantic tuples | Mido semantic tuples | Ratio |
| --- | ---: | ---: | ---: | ---: |
| tiny | 38 | 1.60 µs | 76.8 µs | 48.0× |
| normal | 32,376 | 2.43 ms | 67.5 ms | 27.8× |
| huge | 388,192 | 40.2 ms | 848 ms | 21.1× |
| mahler | 157,980 | 13.5 ms | 345 ms | 25.6× |

This is the primary publishable result because both calls return equal Python
lists containing equal tuples and `bytes` payloads.

## Memory signal

For `mahler.mid`:

- Miso's compact Rust arena reports 2,529,747 heap bytes.
- Mido's retained object graph reports about 38.4 MB through `tracemalloc`.
- The equal semantic tuple list retains about 19.8 MB for either library.
- Miso's tuple conversion peaks at roughly 19.8 MB visible to `tracemalloc`,
  plus the temporary 2.53 MB Rust arena.
- Mido's tuple conversion peaks at roughly 57.8 MB because the Mido graph and
  output tuples coexist during conversion.

Python's `tracemalloc` does not observe Rust allocations, so the Rust arena is
reported separately rather than folded into the traced number.

## Interpretation

The scanner is no longer the bottleneck. Compact parsing takes under one
millisecond for Mahler; creating 157,980 Python tuples takes 13.5 milliseconds.
The next performance work should therefore focus on APIs that avoid per-event
Python objects entirely: iterators backed by native ranges, NumPy-compatible
columns, filtered bulk extraction, and note-level transforms inside Rust.

Raw pyperf data is generated locally at
`benchmarks/results/head-to-head.json`.
