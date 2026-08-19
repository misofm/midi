from __future__ import annotations

import argparse
import hashlib
import json
import struct
import urllib.request
from pathlib import Path

REFERENCE_URL = (
    "https://raw.githubusercontent.com/Yikai-Liao/minimidi/main/example/mahler.mid"
)
REFERENCE_SHA256 = "35a59329ab8f1f86ec2602bb5293b9fbddc694e512aafa00e310cb8da237f302"

PRESETS = {
    "tiny": (1, 16),
    "normal": (8, 2_000),
    "huge": (16, 12_000),
}


def encode_vlq(value: int) -> bytes:
    if not 0 <= value <= 0x0FFF_FFFF:
        raise ValueError("SMF VLQ must fit in 28 bits")
    encoded = bytearray([value & 0x7F])
    value >>= 7
    while value:
        encoded.append(0x80 | (value & 0x7F))
        value >>= 7
    encoded.reverse()
    return bytes(encoded)


def _meta(delta: int, kind: int, payload: bytes) -> bytes:
    return encode_vlq(delta) + bytes((0xFF, kind)) + encode_vlq(len(payload)) + payload


def make_track(track_index: int, notes: int) -> tuple[bytes, int]:
    channel = track_index % 16
    track = bytearray()
    event_count = 0
    running_status: int | None = None

    def channel_event(delta: int, status: int, data: tuple[int, ...]) -> None:
        nonlocal event_count, running_status
        track.extend(encode_vlq(delta))
        if running_status != status:
            track.append(status)
            running_status = status
        track.extend(data)
        event_count += 1

    name = f"generated-{track_index:02d}".encode()
    track.extend(_meta(0, 0x03, name))
    event_count += 1
    channel_event(0, 0xC0 | channel, ((track_index * 7) % 128,))
    # Every generated track has exactly one valid sustain interval.  Keep this
    # separate from the diverse arbitrary-controller stream below so a parser
    # never has to infer an unmatched CC64 transition.
    channel_event(0, 0xB0 | channel, (64, 127))

    delta_pattern = (0, 1, 24, 127, 128, 240, 8_192)
    for index in range(notes):
        note = 24 + ((index * 17 + track_index * 5) % 84)
        velocity = 1 + ((index * 29 + track_index) % 127)
        channel_event(
            delta_pattern[index % len(delta_pattern)],
            0x90 | channel,
            (note, velocity),
        )
        channel_event(24 + index % 97, 0x90 | channel, (note, 0))

        if index % 64 == 0:
            # This bijection retains 119 controller numbers (1..120 except
            # 64) while preserving the old relatively-prime 64/119 cadence.
            controller = 1 + index % 119
            if controller >= 64:
                controller += 1
            channel_event(0, 0xB0 | channel, (controller, index % 128))
        if index % 256 == 0:
            bend = (index * 31) & 0x3FFF
            channel_event(0, 0xE0 | channel, (bend & 0x7F, bend >> 7))
        if index % 512 == 0:
            marker = f"m{index}".encode()
            track.extend(_meta(0, 0x06, marker))
            event_count += 1

    # The zero delta places the CC64 off at the final event time, after the
    # initial time-zero CC64 on and all generated musical content.
    channel_event(0, 0xB0 | channel, (64, 0))
    track.extend(_meta(0, 0x2F, b""))
    event_count += 1
    return bytes(track), event_count


def make_file(track_count: int, notes_per_track: int) -> tuple[bytes, int]:
    chunks = []
    event_count = 0
    for track_index in range(track_count):
        track, track_events = make_track(track_index, notes_per_track)
        chunks.append(b"MTrk" + struct.pack(">I", len(track)) + track)
        event_count += track_events
    header = b"MThd" + struct.pack(">IHHH", 6, 1, track_count, 480)
    return header + b"".join(chunks), event_count


def _smf(tracks: tuple[bytes, ...], *, division: int = 480, smf_format: int = 1) -> bytes:
    """Build a compact format-1 SMF for deterministic contract vectors."""
    if not 0 <= division <= 0xFFFF:
        raise ValueError("SMF division must fit in 16 bits")
    if not 0 <= smf_format <= 2:
        raise ValueError("SMF format must be 0, 1, or 2")
    header = b"MThd" + struct.pack(">IHHH", 6, smf_format, len(tracks), division)
    return header + b"".join(
        b"MTrk" + struct.pack(">I", len(track)) + track for track in tracks
    )


def _end_of_track() -> bytes:
    return b"\x00\xff\x2f\x00"


def score_contract_vectors() -> dict[str, bytes]:
    """Return small, in-memory SMFs for score-construction differentials.

    These vectors intentionally target the observable score contract rather
    than the general raw-event parser.  They are generated rather than stored
    as binary fixtures so their event boundaries remain reviewable here.
    """
    vlq = bytearray()
    # A 1-, 2-, 3-, and 4-byte delta respectively.  Every note is closed so
    # the score contract observes the decoded values as both times and
    # durations, rather than merely as a scanner counter.
    for pitch, delta in zip((60, 61, 62, 63), (0x7F, 0x80, 0x4000, 0x20_0000)):
        vlq.extend(b"\x00\x90" + bytes((pitch, 1)))
        vlq.extend(encode_vlq(delta) + b"\x80" + bytes((pitch, 0)))
    vlq.extend(_end_of_track())

    transitions = bytearray(
        # One-data-byte program-change running status, then two-data-byte
        # note/CC running status, followed by explicit status transitions.
        b"\x00\xc2\x0a\x00\x0b"
        b"\x00\x92\x3c\x64\x01\x3c\x00"
        b"\x00\xb2\x07\x63\x00\x07\x64"
        b"\x00\x92\x3d\x5a\x01\x3d\x00"
        b"\x00\xc2\x01\x00\x92\x3e\x50\x01\x3e\x00"
        # Channel 9 must remain a drum group and follows channel 2 regardless
        # of its earlier/later event position.
        b"\x00\x99\x24\x7f\x01\x24\x00"
    )
    transitions.extend(_end_of_track())

    note_lifecycle = bytearray(
        # Same-pitch overlap closes FIFO.  The following same-tick pair is a
        # valid zero-duration note.  The 62 off is orphaned and the 63 on is
        # dangling at EOT; neither is retained in a score.
        b"\x00\x90\x3c\x0a\x01\x3c\x14"
        b"\x01\x80\x3c\x00\x01\x3c\x00"
        b"\x00\x90\x3d\x1e\x00\x80\x3d\x00"
        b"\x00\x80\x3e\x00\x00\x90\x3f\x2c"
    )
    note_lifecycle.extend(_end_of_track())

    # Keep lyrics out of this Symusic differential.  Symusic 0.6 may split a
    # lyric-only channel-zero group from a same-program note group, whereas
    # Miso's documented policy coalesces both.  The policy is tested below
    # independently; text ordering here is covered by name and marker text.
    globals_first = (
        b"\x00\xff\x03\x02a\xff"  # malformed UTF-8 track name -> a + U+FFFD
        b"\x00\x90\x3c\x01\x01\x80\x3c\x00"
        b"\x00\xff\x51\x03\x07\xa1\x20"  # tick 1, 500000 mspq
        b"\x00\xff\x58\x04\x03\x02\x18\x08"
        b"\x00\xff\x59\x02\xfb\x01"
        b"\x00\xff\x06\x02A\xff"  # marker text -> A + U+FFFD
        + _end_of_track()
    )
    globals_second = (
        # Equal-time tempo remains after the first source track's equal-time
        # tempo; the next delta then checks global time ordering as well.
        b"\x01\xff\x51\x03\x06\x1a\x80"  # tick 1, 400000 mspq
        b"\x04\xff\x51\x03\x05\x16\x15"  # tick 5, 333333 mspq
        b"\x00\xff\x58\x04\x04\x02\x18\x08"
        b"\x00\xff\x59\x02\x01\x00"
        b"\x00\xff\x06\x01y"
        + _end_of_track()
    )

    return {
        "score-vlq-widths.mid": _smf((bytes(vlq),)),
        "score-running-status-groups.mid": _smf((bytes(transitions),)),
        "score-note-lifecycle.mid": _smf((bytes(note_lifecycle),)),
        "score-ordered-globals-text.mid": _smf((globals_first, globals_second)),
    }


def score_policy_vectors() -> dict[str, bytes]:
    """Return score cases governed by Miso policy, not Symusic equality.

    Symusic 0.6 rejects SMPTE divisions outright and has the lyric grouping
    divergence documented in :func:`score_contract_vectors`.  Miso retains the
    raw division field and coalesces channel-zero lyric events with the current
    channel-zero program group.  These cases prevent a competitor limitation
    from silently becoming Miso's product contract.
    """
    smpte_track = b"\x00\x90\x3c\x01\x01\x80\x3c\x00" + _end_of_track()
    lyric_group = (
        b"\x00\x90\x3c\x01\x01\x80\x3c\x00"
        b"\x00\xff\x05\x02l\xff"
        + _end_of_track()
    )
    return {
        # -25 fps and 40 ticks/frame, encoded as an SMF SMPTE division.
        "score-smpte-division.mid": _smf((smpte_track,), division=0xE728),
        "score-lyric-channel-zero-policy.mid": _smf((lyric_group,)),
    }


def score_malformed_files() -> dict[str, tuple[bytes, str]]:
    """Malformed score inputs with stable public error text and offsets.

    General scanner failures stay in :func:`malformed_files`; these two are
    structurally framed SMFs which the score layer rejects because recognised
    meta payloads would otherwise have ambiguous semantics.
    """
    return {
        "score-malformed-tempo-length.mid": (
            _smf((b"\x00\xff\x51\x02\x00\x00" + _end_of_track(),)),
            "SMF parse error at byte 23: meta event 0x51 has invalid payload length 2",
        ),
        "score-malformed-time-signature-denominator.mid": (
            _smf((b"\x00\xff\x58\x04\x04\x40\x18\x08" + _end_of_track(),)),
            "SMF parse error at byte 23: time-signature denominator exponent 64 exceeds u64",
        ),
    }


def score_adversarial_vectors(
    *,
    overlaps: int = 1_024,
    invalid_text_bytes: int = 128,
    global_events: int = 64,
    source_tracks: int = 17,
    events: int = 256,
) -> dict[str, bytes]:
    """Generate bounded adversarial score fixtures without checked-in blobs.

    The defaults are intentionally small enough for a PR correctness run.  The
    same generators admit larger values in a dedicated limit-enforcement test
    once ``ScoreParseLimits`` is public.  They never use a declared chunk size
    as a substitute for the actual generated payload.
    """
    if overlaps < 1:
        raise ValueError("overlaps must be positive")
    if invalid_text_bytes < 1:
        raise ValueError("invalid_text_bytes must be positive")
    if global_events < 1:
        raise ValueError("global_events must be positive")
    if not 1 <= source_tracks <= 0xFFFF:
        raise ValueError("source_tracks must fit in the SMF u16 field")
    if events < 3:
        raise ValueError("events must include one note-on, note-off, and EOT")

    overlap = bytearray(b"\x00\x90\x3c\x01")
    # Fill the FIFO spill queue for a single channel/pitch.  A different
    # velocity makes the queue's output order observable after closure.
    for index in range(1, overlaps):
        overlap.extend(b"\x00\x3c" + bytes((1 + index % 127,)))
    overlap.extend(b"\x00\x80\x3c\x00")
    for _ in range(1, overlaps):
        overlap.extend(b"\x00\x3c\x00")
    overlap.extend(_end_of_track())

    invalid = b"\xff" * invalid_text_bytes
    text = (
        b"\x00\xff\x03" + encode_vlq(len(invalid)) + invalid
        + b"\x00\x90\x3c\x01\x01\x80\x3c\x00"
        + b"\x00\xff\x06" + encode_vlq(len(invalid)) + invalid
        + _end_of_track()
    )

    globals_track = bytearray()
    for index in range(global_events):
        # Equal-time ordered rows expose growth of every global score column.
        globals_track.extend(_meta(0, 0x51, (400_000 + index).to_bytes(3, "big")))
        globals_track.extend(_meta(0, 0x58, bytes((4, 2, 24, 8))))
        globals_track.extend(
            _meta(0, 0x59, bytes((((index % 15) - 7) & 0xFF, index % 2)))
        )
        globals_track.extend(_meta(0, 0x06, f"g{index}".encode()))
    globals_track.extend(_end_of_track())

    max_vlq = b"\x00\x90\x3c\x01" + encode_vlq(0x0FFF_FFFF) + b"\x80\x3c\x00" + _end_of_track()
    # SysEx does not contribute a score row.  The following *explicit* status
    # is the portable, unambiguous path after a system event.
    system = (
        b"\x00\x90\x3c\x01\x00\xf0\x03\x01\x02\xf7"
        b"\x01\x80\x3c\x00"
        + _end_of_track()
    )

    event_density = bytearray(b"\x00\x90\x3c\x01")
    # Every one of these physical events is semantic output, so a future
    # max_events check cannot be delayed until after an ignored event stream.
    for index in range(events - 3):
        event_density.extend(b"\x00\xb0" + bytes((index % 128, index % 128)))
    event_density.extend(b"\x01\x80\x3c\x00")
    event_density.extend(_end_of_track())

    # These policy vectors are intentionally not asserted as current parity:
    # they are fixtures for strict EOT/running-status enforcement.  The matrix
    # names their proposed error offsets and compatibility treatment.
    missing_eot = b"\x00\x90\x3c\x01\x01\x80\x3c\x00"
    after_eot = _end_of_track() + b"\x00\x90\x3c\x01\x01\x80\x3c\x00"
    running_after_sysex = (
        b"\x00\x90\x3c\x01\x00\xf0\x01\x7f\x01\x3c\x00" + _end_of_track()
    )
    running_after_f7 = (
        b"\x00\x90\x3c\x01\x00\xf7\x01\x7f\x01\x3c\x00" + _end_of_track()
    )
    running_after_meta = (
        b"\x00\x90\x3c\x01\x00\xff\x01\x01x\x01\x3c\x00" + _end_of_track()
    )
    valid_f7 = (
        b"\x00\x90\x3c\x01\x00\xf7\x01\x7f\x01\x80\x3c\x00" + _end_of_track()
    )
    zero_text_meta = (
        b"\x00\xff\x03\x00\x00\x90\x3c\x01\x01\x80\x3c\x00"
        b"\x00\xff\x06\x00"
        + _end_of_track()
    )
    note = b"\x00\x90\x3c\x01\x01\x80\x3c\x00" + _end_of_track()

    return {
        "score-overlap-queue.mid": _smf((bytes(overlap),)),
        "score-invalid-text-expansion.mid": _smf((text,)),
        "score-global-density.mid": _smf((bytes(globals_track),)),
        "score-max-vlq.mid": _smf((max_vlq,)),
        "score-system-event-continuation.mid": _smf((system,)),
        "score-event-density.mid": _smf((bytes(event_density),)),
        "score-source-tracks.mid": _smf(tuple(_end_of_track() for _ in range(source_tracks))),
        "score-missing-eot-policy.mid": _smf((missing_eot,)),
        "score-events-after-eot-policy.mid": _smf((after_eot,)),
        "score-running-after-sysex-policy.mid": _smf((running_after_sysex,)),
        "score-running-after-f7-policy.mid": _smf((running_after_f7,)),
        "score-running-after-meta-policy.mid": _smf((running_after_meta,)),
        "score-valid-f7.mid": _smf((valid_f7,)),
        "score-zero-text-meta.mid": _smf((zero_text_meta,)),
        "score-format-0.mid": _smf((note,), smf_format=0),
        "score-format-2.mid": _smf((note,), smf_format=2),
        "score-zero-division-policy.mid": _smf((note,), division=0),
        "score-invalid-smpte-division-policy.mid": _smf((note,), division=0x8000),
    }


def score_resource_header_vectors() -> dict[str, bytes]:
    """Return no-allocation header/chunk declaration boundary cases.

    The declared values are deliberately not materialised.  These fixtures
    exercise early header and chunk admission before a future parser allocates
    for a track or score arena.
    """
    return {
        "score-declared-u32-track.mid": (
            b"MThd\0\0\0\x06\0\x01\0\x01\x01\xe0MTrk\xff\xff\xff\xff"
        ),
        "score-truncated-track-boundary.mid": (
            b"MThd\0\0\0\x06\0\x01\0\x01\x01\xe0MTrk\0\0\0\x04\0\xff\x2f"
        ),
    }


def score_running_status_cancellation_vectors() -> dict[str, tuple[bytes, int]]:
    """Malformed runs which must fail after F0, F7, or FF clears status.

    Offsets are absolute in their fully framed one-track SMFs.  They remain
    separate from the core's decoder unit tests so Python scan, raw arena, and
    score entry points share one public regression contract.
    """
    vectors = score_adversarial_vectors()
    return {
        "score-running-after-sysex-policy.mid": (
            vectors["score-running-after-sysex-policy.mid"],
            31,
        ),
        "score-running-after-f7-policy.mid": (
            vectors["score-running-after-f7-policy.mid"],
            31,
        ),
        "score-running-after-meta-policy.mid": (
            vectors["score-running-after-meta-policy.mid"],
            32,
        ),
    }


def malformed_files() -> dict[str, bytes]:
    valid, _ = make_file(1, 1)
    track_start = 22
    return {
        "malformed-truncated-header.mid": b"MThd\x00\x00",
        "malformed-truncated-track.mid": valid[:-2],
        "malformed-missing-running-status.mid": (
            b"MThd\0\0\0\x06\0\x01\0\x01\x01\xe0"
            b"MTrk\0\0\0\x03\0\x3c\x40"
        ),
        "malformed-vlq-too-long.mid": (
            b"MThd\0\0\0\x06\0\x01\0\x01\x01\xe0"
            b"MTrk\0\0\0\x08\x80\x80\x80\x80\0\xff\x2f\0"
        ),
        "malformed-invalid-data.mid": (
            valid[:track_start] + b"\0\x90\x80\x40" + valid[track_start + 4 :]
        ),
        # A declared u32-sized chunk must be rejected from its declared
        # boundary without allocating based on untrusted input.
        "malformed-oversized-track-declaration.mid": (
            b"MThd\0\0\0\x06\0\x01\0\x01\x01\xe0"
            b"MTrk\xff\xff\xff\xff"
        ),
        "malformed-invalid-status.mid": (
            b"MThd\0\0\0\x06\0\x01\0\x01\x01\xe0"
            b"MTrk\0\0\0\x02\0\xf1"
        ),
    }


def _write(path: Path, data: bytes) -> dict[str, int | str]:
    path.write_bytes(data)
    return {
        "bytes": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
    }


def fetch_reference(output: Path) -> dict[str, int | str]:
    with urllib.request.urlopen(REFERENCE_URL, timeout=30) as response:
        data = response.read()
    digest = hashlib.sha256(data).hexdigest()
    if digest != REFERENCE_SHA256:
        raise RuntimeError(f"mahler.mid checksum changed: {digest}")
    return _write(output / "mahler.mid", data)


def generate(output: Path, include_reference: bool = True) -> dict[str, object]:
    output.mkdir(parents=True, exist_ok=True)
    manifest: dict[str, object] = {"generated": {}, "malformed": {}}

    for name, (tracks, notes) in PRESETS.items():
        data, event_count = make_file(tracks, notes)
        record = _write(output / f"{name}.mid", data)
        record.update({"tracks": tracks, "events": event_count})
        manifest["generated"][f"{name}.mid"] = record

    for name, data in malformed_files().items():
        manifest["malformed"][name] = _write(output / name, data)

    if include_reference:
        manifest["reference"] = {
            "mahler.mid": {
                **fetch_reference(output),
                "source": REFERENCE_URL,
            }
        }

    (output / "manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    )
    return manifest


def refresh_generated(output: Path) -> dict[str, object]:
    """Refresh only deterministic generated files, preserving other corpus data.

    This is intentionally narrower than ``generate(..., include_reference=False)``:
    malformed fixtures and the checksum-pinned downloaded reference retain both
    their bytes and their manifest entries.
    """
    manifest_path = output / "manifest.json"
    if not manifest_path.is_file():
        raise FileNotFoundError(f"cannot refresh generated corpus without {manifest_path}")
    manifest = json.loads(manifest_path.read_text())
    generated: dict[str, dict[str, int | str]] = {}
    for name, (tracks, notes) in PRESETS.items():
        data, event_count = make_file(tracks, notes)
        record = _write(output / f"{name}.mid", data)
        record.update({"tracks": tracks, "events": event_count})
        generated[f"{name}.mid"] = record
    manifest["generated"] = generated
    manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    return manifest


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate the MIDI benchmark corpus")
    parser.add_argument(
        "--output", type=Path, default=Path(__file__).with_name("corpus")
    )
    parser.add_argument(
        "--reference",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="download the checksum-pinned mahler.mid reference file",
    )
    parser.add_argument(
        "--generated-only",
        action="store_true",
        help="refresh only tiny/normal/huge and their manifest entries",
    )
    args = parser.parse_args()
    manifest = (
        refresh_generated(args.output)
        if args.generated_only
        else generate(args.output, include_reference=args.reference)
    )
    print(json.dumps(manifest, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
