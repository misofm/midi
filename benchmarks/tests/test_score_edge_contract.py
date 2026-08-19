"""Small, deterministic score-construction contracts around SMF edge cases."""

from __future__ import annotations

import pytest
from symusic import Score as SymusicScore

from benchmarks.corpus import (
    malformed_files,
    score_contract_vectors,
    score_malformed_files,
    score_policy_vectors,
)
from benchmarks.score_contract import score_contract_from_records, symusic_score_contract
from miso_midi import parse, parse_score, scan


def _miso_records(data: bytes) -> dict[str, object]:
    return parse_score(data).semantic_records()


@pytest.mark.parametrize("name", score_contract_vectors(), ids=lambda name: name)
def test_score_edge_vectors_match_symusic_060_contract(name: str) -> None:
    """Differential coverage for supported, unambiguous Symusic 0.6 inputs."""
    data = score_contract_vectors()[name]
    actual = score_contract_from_records(_miso_records(data))
    expected = symusic_score_contract(SymusicScore.from_midi(data))

    assert actual == expected


def test_one_to_four_byte_vlqs_are_observable_as_exact_note_times() -> None:
    data = score_contract_vectors()["score-vlq-widths.mid"]
    notes = _miso_records(data)["tracks"][0]["notes"]  # type: ignore[index]

    assert [(note["time"], note["duration"]) for note in notes] == [  # type: ignore[index]
        (0, 0x7F),
        (0x7F, 0x80),
        (0x7F + 0x80, 0x4000),
        (0x7F + 0x80 + 0x4000, 0x20_0000),
    ]
    assert scan(data).max_delta_ticks == 0x20_0000


def test_running_status_transitions_preserve_channel_program_group_order() -> None:
    records = _miso_records(score_contract_vectors()["score-running-status-groups.mid"])
    tracks = records["tracks"]  # type: ignore[index]

    assert [(track["program"], track["is_drum"]) for track in tracks] == [  # type: ignore[index]
        (1, False),
        (11, False),
        (0, True),
    ]
    assert tracks[1]["notes"] == [  # type: ignore[index]
        {"time": 0, "duration": 1, "pitch": 60, "velocity": 100},
        {"time": 1, "duration": 1, "pitch": 61, "velocity": 90},
    ]
    assert tracks[1]["controls"] == [  # type: ignore[index]
        {"time": 1, "number": 7, "value": 99},
        {"time": 1, "number": 7, "value": 100},
    ]


def test_overlapping_notes_are_fifo_and_zero_orphan_dangling_cases_are_explicit() -> None:
    records = _miso_records(score_contract_vectors()["score-note-lifecycle.mid"])
    notes = records["tracks"][0]["notes"]  # type: ignore[index]

    assert notes == [
        {"time": 0, "duration": 2, "pitch": 60, "velocity": 10},
        {"time": 1, "duration": 2, "pitch": 60, "velocity": 20},
        {"time": 3, "duration": 0, "pitch": 61, "velocity": 30},
    ]


def test_global_event_and_malformed_text_ordering_is_stable() -> None:
    records = _miso_records(score_contract_vectors()["score-ordered-globals-text.mid"])

    assert records["tracks"][0]["name"] == "a\ufffd"  # type: ignore[index]
    assert records["markers"] == [  # type: ignore[index]
        {"time": 1, "text": "A\ufffd"},
        {"time": 5, "text": "y"},
    ]
    assert records["tempos"] == [  # type: ignore[index]
        {"time": 1, "mspq": 500_000},
        {"time": 1, "mspq": 400_000},
        {"time": 5, "mspq": 333_333},
    ]
    assert records["time_signatures"] == [  # type: ignore[index]
        {"time": 1, "numerator": 3, "denominator": 4},
        {"time": 5, "numerator": 4, "denominator": 4},
    ]
    assert records["key_signatures"] == [  # type: ignore[index]
        {"time": 1, "key": -5, "tonality": 1},
        {"time": 5, "key": 1, "tonality": 0},
    ]


def test_smpte_and_lyric_grouping_follow_documented_miso_policy() -> None:
    """Do not substitute Symusic 0.6's unsupported/divergent behaviour.

    Symusic rejects SMPTE division input, and splits this lyric-only
    channel-zero content into a separate same-program track.  Miso's written
    policy retains the raw division and coalesces compatible channel-zero
    content, so neither case is a differential oracle.
    """
    vectors = score_policy_vectors()
    smpte = vectors["score-smpte-division.mid"]
    lyric = vectors["score-lyric-channel-zero-policy.mid"]

    assert scan(smpte).division == 0xE728
    assert _miso_records(smpte)["tpq"] == 0xE728
    with pytest.raises(RuntimeError, match="Division type is not ticks per quarter"):
        SymusicScore.from_midi(smpte)

    lyric_records = _miso_records(lyric)
    assert len(lyric_records["tracks"]) == 1  # type: ignore[arg-type]
    assert lyric_records["tracks"][0]["lyrics"] == [{"time": 1, "text": "l\ufffd"}]  # type: ignore[index]


_RAW_MALFORMED_ERRORS = {
    "malformed-truncated-header.mid": "SMF parse error at byte 4: unexpected end of input",
    "malformed-truncated-track.mid": "SMF parse error at byte 22: unexpected end of input",
    "malformed-missing-running-status.mid": (
        "SMF parse error at byte 23: data byte encountered without channel running status"
    ),
    "malformed-vlq-too-long.mid": (
        "SMF parse error at byte 22: variable-length quantity exceeds four bytes"
    ),
    "malformed-invalid-data.mid": (
        "SMF parse error at byte 24: channel data byte 0x80 has its status bit set"
    ),
    "malformed-oversized-track-declaration.mid": (
        "SMF parse error at byte 22: unexpected end of input"
    ),
    "malformed-invalid-status.mid": (
        "SMF parse error at byte 23: status 0xf1 is not valid in an SMF track"
    ),
}


@pytest.mark.parametrize("name", _RAW_MALFORMED_ERRORS, ids=lambda name: name)
def test_raw_malformed_and_resource_boundaries_have_stable_errors(name: str) -> None:
    data = malformed_files()[name]
    expected = _RAW_MALFORMED_ERRORS[name]

    for parser in (scan, parse, parse_score):
        with pytest.raises(ValueError) as error:
            parser(data)
        if name == "malformed-oversized-track-declaration.mid" and parser is parse_score:
            assert str(error.value) == (
                "SMF parse error at byte 18: score parse limit exceeded: "
                "track bytes (limit 16777216)"
            )
        else:
            assert str(error.value) == expected


@pytest.mark.parametrize("name", score_malformed_files(), ids=lambda name: name)
def test_score_recognized_meta_policy_has_stable_errors(name: str) -> None:
    data, expected = score_malformed_files()[name]

    # These bytes are structurally framed SMFs.  The score parser rejects them
    # strictly because no score-level interpretation is defined for them.
    scan(data)
    parse(data)
    with pytest.raises(ValueError) as error:
        parse_score(data)
    assert str(error.value) == expected
