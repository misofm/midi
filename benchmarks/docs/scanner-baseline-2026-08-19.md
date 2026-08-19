# Baseline — 2026-08-19

This is an early architecture-spike result, not a release claim. Miso performs
a complete structural event scan and returns one summary object. Mido performs
that scan and eagerly materializes every Python message and track object.

## Environment

- AMD Ryzen 7 9700X, 8 cores / 16 threads
- Linux 6.8.0-138, x86-64, glibc 2.39
- CPython 3.12.3
- Rust 1.97.1; release extension with thin LTO
- Mido 1.3.3, pyperf 2.10.0
- CPU governor reported by pyperf as `powersave`
- 3 worker processes, 3 values, 1 warmup, 50 ms minimum value time

## In-memory results

| Corpus | Bytes | Events | `miso/scan` | `mido/materialize` |
| --- | ---: | ---: | ---: | ---: |
| tiny | 163 | 38 | 108 ns ± 3 ns | 65.0 µs ± 0.3 µs |
| normal | 104,910 | 32,376 | 51.9 µs ± 0.8 µs | 58.0 ms ± 1.1 ms |
| huge | 1,256,302 | 388,192 | 617 µs ± 5 µs | 737 ms ± 4 ms |
| mahler | 656,425 | 157,980 | 327 µs ± 5 µs | 297 ms ± 3 ms |

The `mahler.mid` checksum is
`35a59329ab8f1f86ec2602bb5293b9fbddc694e512aafa00e310cb8da237f302`.
On this machine Mido is roughly ten times faster than the widely quoted
2.92-second result, which is why machine and workload details must accompany
every comparison.

A single `tracemalloc` pass while retaining the result reported about 38.4 MB
for Mido's materialized `mahler` graph and 80 bytes for Miso's summary handle.
These represent different output layers and must not be presented as an
equal-work memory comparison.

## Profile signal

`cProfile` recorded 6.39 million Python calls while Mido loaded `mahler.mid`.
The largest cumulative paths were per-track parsing, message construction and
decoding, data-byte checks, and 656,147 calls to `read_byte`. This supports the
bulk-decoder direction, but the next honest comparison requires Miso's owned
event arena and equivalent semantic validation.

Raw pyperf data is generated locally at `benchmarks/results/baseline.json`.
