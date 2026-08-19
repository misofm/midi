# Architecture

## Product boundary

Miso MIDI owns wire-level MIDI semantics and efficient in-memory
representations. Language SDKs own idiomatic objects, iteration, NumPy or typed
array integration, and convenience transforms. Miso Engine consumes the Rust
crates directly; it does not call through FFI.

The library should not copy another library's API or make its internal object graph the
canonical model. Compatibility adapters may exist later, but they are leaf
packages rather than core constraints.

## Crate trajectory

Start with one small core and split only when the contracts are proven:

```text
miso-midi-core
  SMF wire decoder/encoder
  shared message and error vocabulary
  borrowed event views
  owned compact arena
  incremental live decoder

adapter crates
  miso-midi-python  -> PyO3 + maturin
  miso-midi-node    -> napi-rs
  miso-midi-wasm    -> wasm-bindgen
  miso-midi-c       -> optional versioned C ABI
```

If live decoding and SMF evolve incompatible allocation or feature needs, split
them into `miso-midi-live`, `miso-midi-smf`, and `miso-midi-types`. Splitting on
day one would create versioning work before the shared vocabulary is known.

## Representations

No single representation wins every workload. The core should offer three
layers over one decoder:

1. `EventIter<'a>`: borrowed, lazy, zero-allocation event traversal over an
   input byte slice.
2. `EventArena`: compact owned storage with track ranges, fixed-size event
   headers, and a byte arena for variable payloads.
3. SDK views: lightweight Python or JavaScript handles backed by the arena,
   plus bulk structure-of-arrays exports for analytical workloads.

The FFI boundary must be coarse. Parsing a file in Rust and then making one FFI
call per event forfeits much of the win. SDKs should request a whole arena,
track, batch, or typed column at a time.

## Parsing modes

- `strict`: standards-conforming validation with stable error code and byte
  offset.
- `compatible`: explicitly documented recovery for common real-world files.
- `lossless`: preserves unknown chunks, unknown meta events, original payloads,
  and enough encoding information for fidelity-oriented writing.
- `normalized`: canonical semantic events optimized for transforms and engine
  scheduling.

Limits for tracks, events, chunk sizes, VLQ length, and SysEx payloads are
explicit inputs. Malformed files must fail in bounded time without panic or
unbounded allocation.

## Engine boundary

File parsing is preparation work and may build an owned arena. Before playback,
SMF events are transformed into the engine's admitted, bounded event schedule.
The audio callback never parses files or allocates.

Live MIDI is a separate fixed-state decoder. It accepts bytes or UMP words and
emits into caller-provided bounded storage. Overflow and malformed input have
machine-readable outcomes. MIDI events are not smuggled into the existing audio
parameter-event vocabulary; the engine's MIDI instrument graph defines that
later contract.

## Safety and performance policy

- Begin with `#![forbid(unsafe_code)]`; permit narrowly reviewed unsafe blocks
  only when benchmarks demonstrate a material gain that safe code cannot
  recover.
- Keep the wire core dependency-free and `no_std` capable.
- Validate first with unit, property, corpus, differential, and fuzz tests.
- Benchmark release artifacts on pinned corpora and report throughput, latency,
  allocations, and peak memory.
- Compare equal semantic layers. Scanner versus object materialization is useful
  diagnostic data but not an equal-work speedup claim.

## SDK strategy

Python uses PyO3 and maturin, managed locally with `uv`. Stable-ABI wheels reduce
the wheel matrix, subject to measured performance and free-threaded Python
support.

Node uses napi-rs for native packages and typed-buffer ownership. The browser
uses wasm-bindgen. A single TypeScript facade can choose the native Node backend
or browser Wasm backend while keeping observable semantics aligned.

A versioned C ABI is useful for Swift, Kotlin/Native, C++, or plugin hosts. It
should expose opaque handles, borrowed buffers, explicit ownership functions,
and numeric error codes. It is not the internal architecture.
