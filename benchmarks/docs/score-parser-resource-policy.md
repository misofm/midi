# Score-parser resource and malformed-input policy

The machine-readable [parity matrix](score-parser-parity-matrix.json) is the
M1 admission checklist. Rust provides a checked score-parser entry point, and
the Python score SDK is finite by default. The legacy Rust fast path and an
explicit Python escape hatch remain unlimited only for trusted input.

## Implemented Rust API

`miso-midi-core` implements `ScoreParseLimits`, `ScoreResource`,
`ParseErrorKind::ResourceLimitExceeded`, and
`parse_score_smf_with_limits(data, limits)`. The checked entry point is the
Rust API for untrusted bytes. `parse_score_smf(data)` intentionally remains the
trusted-input unlimited fast path and can exhaust memory on hostile input.

```rust
pub struct ScoreParseLimits {
    pub max_input_bytes: usize,
    pub max_source_tracks: u16,
    pub max_track_bytes: usize,
    pub max_events: usize,
    pub max_note_starts: usize,
    pub max_text_bytes: usize,
}

pub fn parse_score_smf_with_limits(
    data: &[u8],
    limits: ScoreParseLimits,
) -> Result<TickScore, ParseError>;
```

The score-specific resources are `InputBytes`, `SourceTracks`,
`TrackBytes`, `Events`, `NoteStarts`, and `TextBytes`. `max_events` counts
every physical MTrk event and therefore bounds global-event rows, score groups,
the overlap spill queue, and stable-sort input without adding subtly different
limits for each representation. Internal arena ranges still require checked
`u32` conversions regardless of configured limits.

## Finite defaults and Python API

`ScoreParseLimits::DEFAULT` is 64 MiB input, 4,096 source tracks, 16 MiB per
track, 2,000,000 physical events, 1,000,000 note starts, and 16 MiB normalized
text. Python exposes the same values through a frozen public
`ScoreParseLimits` class:

```python
parse_score(data, *, limits: ScoreParseLimits | None = None,
            mode: Literal["compatible", "strict"] = "compatible")
Score(data, *, limits: ScoreParseLimits | None = None,
      mode: Literal["compatible", "strict"] = "compatible")
```

`limits=None` means these finite defaults; it never means unlimited. Callers
may customize any ceiling but cannot bypass checked internal `u32` arena
conversions. `parse_score_unlimited(data)` is the only trusted/legacy escape
hatch. It bypasses every logical ceiling, is unsafe for hostile bytes, and must
not be used as a network or upload parser.

## Error precedence and offsets

The implemented Rust error is
`ParseErrorKind::ResourceLimitExceeded { resource, limit }`, formatted as:

```text
SMF parse error at byte <offset>: score parse limit exceeded: <resource description> (limit <limit>)
```

The first invalid byte wins. The implemented checked parser checks input,
header count, and each declared chunk before allocating from them, then checks
event/note/text limits before appending the triggering item. The offset is
respectively byte 0, the header field, the `MTrk` length field, or the
triggering event's delta-VLQ start. Python preserves this exact Rust text in
`ValueError`.

`TickOverflow` remains a distinct error at the delta-VLQ start; it is not a
resource-limit spelling. Any internal range that would exceed `u32::MAX` must
fail before its append with stable `SizeOverflow` behavior (or an earlier
finite event/note/text limit), never through a failed allocation or a
truncated range.

## Strict versus compatible grammar

Resource limits apply independently of grammar policy. Python exposes the same
finite limits in both modes. Compatibility concerns semantic recovery, not
denial-of-service admission:

| Case | Compatible | Strict |
| --- | --- | --- |
| orphan note-off | discard | discard (score policy) |
| dangling note-on at track end | discard | discard (score policy) |
| missing EOT | accept chunk end | `MissingEndOfTrack` at declared track end |
| second EOT or event after EOT | stop/ignore after first EOT; raw lossless parse retains bytes separately | `EventAfterEndOfTrack` at that event's delta offset |
| data byte after F0/F7 | reject once system events clear running status | reject once system events clear running status |
| bytes after all declared tracks | expose `bytes_consumed`/`trailing_bytes` | `TrailingBytes` at `bytes_consumed` |

EOT is structural in strict mode: payload length must be zero, exactly one EOT
is required per declared `MTrk`, and parsing stops at it. This is deliberately
not based on Symusic's malformed-file behavior. `score-running-after-sysex-
policy.mid` is the minimal regression fixture for clearing channel running
status after F0/F7. The minimal fixture's offending data byte is absolute byte
31 (track-local offset 9) under normal 14-byte header plus 8-byte track
framing; `score-running-after-meta-policy.mid` exercises FF and fails at
absolute byte 32. Default compatible parsing accepts missing EOT and
stops/ignores after the first EOT; `parse_score_unlimited` retains legacy
post-EOT semantic events.

For division, strict mode must reject TPQ zero and invalid SMPTE frame codes;
compatible tick-score parsing may retain the raw field for observation. This
does not rely on Symusic: it rejects all SMPTE division, while Mido exposes an
invalid `0x8000` division as `-32768` and both accept TPQ zero.

## Corpus use

The generators in `benchmarks/corpus.py` make small default adversarial files
for PR tests and accept parameters for limit-specific test runs. Never create a
near-`u32` binary fixture merely to prove a range check: use its declared-size
or formula-level admission test, and assert rejection before allocation.
