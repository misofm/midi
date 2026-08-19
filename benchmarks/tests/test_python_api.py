from io import BytesIO
from pathlib import Path

import mido
import pytest

from benchmarks.corpus import make_file, malformed_files
from miso_midi import MidiFile, load, parse, scan


def _skip_vlq(data: bytes, position: int) -> int:
    while data[position] & 0x80:
        position += 1
    return position + 1


def mido_semantic_records(midi: mido.MidiFile) -> list[tuple]:
    records = []
    for track_index, track in enumerate(midi.tracks):
        for message in track:
            if message.is_meta:
                encoded = bytes(message.bytes())
                payload_start = _skip_vlq(encoded, 2)
                records.append(
                    (track_index, message.time, 0xFF, encoded[1], encoded[payload_start:])
                )
            elif message.type == "sysex":
                records.append(
                    (track_index, message.time, 0xF0, None, bytes(message.data))
                )
            else:
                encoded = bytes(message.bytes())
                records.append(
                    (track_index, message.time, encoded[0], None, encoded[1:])
                )
    return records


@pytest.mark.parametrize("tracks,notes", [(1, 1), (3, 20), (8, 250)])
def test_scan_counts_match_mido_materialization(tracks: int, notes: int) -> None:
    data, expected_events = make_file(tracks, notes)
    summary = scan(data)
    midi = mido.MidiFile(file=BytesIO(data))

    assert summary.format == 1
    assert summary.tracks == tracks
    assert summary.events == expected_events
    assert summary.events == sum(len(track) for track in midi.tracks)
    assert summary.bytes_consumed == len(data)
    assert summary.trailing_bytes == 0


@pytest.mark.parametrize("tracks,notes", [(1, 1), (3, 20), (8, 250)])
def test_owned_arena_matches_mido_semantic_records(tracks: int, notes: int) -> None:
    data, expected_events = make_file(tracks, notes)
    ours = parse(data)
    theirs = mido.MidiFile(file=BytesIO(data))

    assert isinstance(ours, MidiFile)
    assert ours.format == theirs.type
    assert ours.track_count == len(theirs.tracks)
    assert ours.division == theirs.ticks_per_beat
    assert ours.event_count == expected_events
    assert len(ours) == expected_events
    assert ours.track_lengths == [len(track) for track in theirs.tracks]
    assert ours.semantic_records() == mido_semantic_records(theirs)


def test_sysex_normalization_matches_mido() -> None:
    track = bytes(
        [
            0x00,
            0xF0,
            0x03,
            1,
            2,
            0xF7,
            0x00,
            0xFF,
            0x2F,
            0x00,
        ]
    )
    data = (
        b"MThd\0\0\0\x06\0\x01\0\x01\x01\xe0"
        + b"MTrk"
        + len(track).to_bytes(4, "big")
        + track
    )
    ours = parse(data).semantic_records()
    theirs = mido_semantic_records(mido.MidiFile(file=BytesIO(data)))
    assert ours == theirs
    assert ours[0] == (0, 0, 0xF0, None, b"\x01\x02")


def test_load_reads_from_path(tmp_path: Path) -> None:
    data, expected_events = make_file(2, 4)
    path = tmp_path / "fixture.mid"
    path.write_bytes(data)
    assert load(path).event_count == expected_events


@pytest.mark.skipif(
    not Path("benchmarks/corpus/mahler.mid").exists(),
    reason="reference corpus has not been generated",
)
def test_mahler_reference_matches_mido_semantic_records() -> None:
    data = Path("benchmarks/corpus/mahler.mid").read_bytes()
    ours = parse(data)
    theirs = mido.MidiFile(file=BytesIO(data))
    assert ours.event_count == 157_980
    assert ours.semantic_records() == mido_semantic_records(theirs)


@pytest.mark.parametrize("name,data", malformed_files().items())
def test_malformed_inputs_are_rejected(name: str, data: bytes) -> None:
    with pytest.raises(ValueError, match="SMF parse error"):
        scan(data)
    with pytest.raises(ValueError, match="SMF parse error"):
        parse(data)
