from __future__ import annotations

from copy import deepcopy
from io import BytesIO

import mido
import numpy as np
import pytest
from symusic import Score

from benchmarks.corpus import make_file
from benchmarks.score_contract import (
    SCORE_CONTRACT_SCHEMA,
    miso_score_contract,
    score_contract_from_records,
    semantic_records_from_symusic,
    symusic_score_contract,
)


@pytest.mark.parametrize(
    ("tracks", "notes_per_track", "expected"),
    [
        (
            1,
            16,
            {
                "tracks": 1,
                "notes": 16,
                "controls": 3,
                "pitch_bends": 1,
                "pedals": 1,
                "lyrics": 0,
                "time_signatures": 0,
                "key_signatures": 0,
                "tempos": 0,
                "markers": 1,
            },
        ),
        (
            8,
            2_000,
            {
                "tracks": 8,
                "notes": 16_000,
                "controls": 272,
                "pitch_bends": 64,
                # Each source track contains one explicit matched CC64
                # sustain interval; the periodic controller stream excludes
                # CC64 entirely.
                "pedals": 8,
                "lyrics": 0,
                "time_signatures": 0,
                "key_signatures": 0,
                "tempos": 0,
                "markers": 32,
            },
        ),
    ],
)
def test_symusic_generated_fixture_counts(
    tracks: int, notes_per_track: int, expected: dict[str, int]
) -> None:
    data, _ = make_file(tracks, notes_per_track)
    contract = symusic_score_contract(Score.from_midi(data))

    assert contract.summary.as_dict() == expected
    assert len(contract.sha256) == 64
    assert contract.metadata()["contract_schema"] == SCORE_CONTRACT_SCHEMA


@pytest.mark.parametrize("tracks,notes_per_track", [(1, 1), (3, 20), (8, 2_000)])
def test_generated_sustain_transitions_are_intentional_and_matched(
    tracks: int, notes_per_track: int
) -> None:
    data, _ = make_file(tracks, notes_per_track)
    midi = mido.MidiFile(file=BytesIO(data))

    for track in midi.tracks:
        absolute_time = 0
        sustain = []
        for message in track:
            absolute_time += message.time
            if message.type == "control_change" and message.control == 64:
                sustain.append((absolute_time, message.value))

        # The generator emits only its deliberate CC64 on/off pair.  The off
        # is later than tick zero and no arbitrary-controller event can be
        # mistaken for a sustain transition.
        assert sustain[0] == (0, 127)
        assert sustain[1][0] > 0
        assert sustain[1][1] == 0
        assert len(sustain) == 2


def test_contract_is_deterministic_and_binary_not_repr() -> None:
    data, _ = make_file(1, 16)
    first = symusic_score_contract(Score.from_midi(data))
    second = symusic_score_contract(Score.from_midi(data))

    assert first == second
    assert first.canonical.startswith(b"MISO-SCORE-CONTRACT\x00\x01")
    assert first.sha256 != __import__("hashlib").sha256(repr(semantic_records_from_symusic(Score.from_midi(data))).encode()).hexdigest()


def test_contract_is_sensitive_to_note_content() -> None:
    data, _ = make_file(1, 16)
    original = symusic_score_contract(Score.from_midi(data))
    changed_score = Score.from_midi(data)
    changed_score.tracks[0].notes[0].velocity += 1
    changed = symusic_score_contract(changed_score)

    assert changed.summary == original.summary
    assert changed.sha256 != original.sha256
    assert changed.canonical != original.canonical


def test_records_round_trip_through_canonical_contract() -> None:
    data, _ = make_file(1, 16)
    score = Score.from_midi(data)
    expected = symusic_score_contract(score)
    records = semantic_records_from_symusic(score)

    # The future Miso bulk semantic_records() protocol is deliberately made of
    # normal builtins; there is no event-object requirement at the FFI edge.
    assert score_contract_from_records(deepcopy(records)) == expected


def test_contract_normalizes_numpy_integer_scalars() -> None:
    data, _ = make_file(1, 16)
    expected = symusic_score_contract(Score.from_midi(data))
    records = semantic_records_from_symusic(Score.from_midi(data))
    records["tpq"] = np.int64(records["tpq"])
    records["tracks"][0]["notes"][0]["velocity"] = np.int16(
        records["tracks"][0]["notes"][0]["velocity"]
    )

    assert score_contract_from_records(records) == expected


def test_miso_adapter_uses_the_bulk_score_protocol() -> None:
    data, _ = make_file(1, 1)
    assert miso_score_contract(data) == symusic_score_contract(Score.from_midi(data))
