"""Audit fixtures for M1 score-parser breadth and future limit enforcement."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from benchmarks.corpus import (
    score_adversarial_vectors,
    score_contract_vectors,
    score_policy_vectors,
    score_resource_header_vectors,
    score_running_status_cancellation_vectors,
)
from miso_midi import parse, parse_score, parse_score_unlimited, scan


_MATRIX_PATH = Path(__file__).resolve().parents[1] / "docs" / "score-parser-parity-matrix.json"


def test_score_parser_parity_matrix_is_machine_readable_and_fixture_backed() -> None:
    matrix = json.loads(_MATRIX_PATH.read_text())
    assert matrix["schema"] == "miso-score-parser-parity/v1"
    assert matrix["reference"] == {  # Pin the differential admission oracle.
        "name": "Symusic",
        "version": "0.6.0",
        "role": "differential oracle only for supported, well-formed tick-score inputs",
    }

    semantic_ids = [row["id"] for row in matrix["semantics"]]
    resource_ids = [row["id"] for row in matrix["resource_limits"]]
    assert len(semantic_ids) == len(set(semantic_ids))
    assert len(resource_ids) == len(set(resource_ids))
    assert {"smpte_division", "lyrics_channel_zero_grouping", "eot_and_file_trailing"} <= set(
        semantic_ids
    )
    assert {
        "input_bytes",
        "source_tracks",
        "track_bytes",
        "events",
        "notes",
        "text_bytes",
        "global_events",
        "overlap_queue",
        "absolute_tick",
        "score_arena_u32_ranges",
    } <= set(resource_ids)
    assert matrix["parser_modes"]["checked_rust"].startswith("implemented:")
    assert matrix["parser_modes"]["python_default"].startswith("implemented:")
    assert "finite defaults" in matrix["error_contract"]["python"]
    assert all("default" in row for row in matrix["resource_limits"])
    semantics = {row["id"]: row for row in matrix["semantics"]}
    for identifier, offset in (("running_status_after_sysex", 31), ("running_status_after_meta", 32)):
        row = semantics[identifier]
        assert row["miso"] == "implemented_spec_policy"
        assert f"byte {offset}" in row["current_observation"]
        assert "current Miso accepts" not in row["current_observation"]

    fixture_names = (
        set(score_contract_vectors())
        | set(score_policy_vectors())
        | set(score_adversarial_vectors())
        | set(score_resource_header_vectors())
        | set(score_running_status_cancellation_vectors())
    )
    fixture_text = "\n".join(
        str(row.get("fixture", "")) for row in (*matrix["semantics"], *matrix["resource_limits"])
    )
    for name in fixture_names:
        assert name in fixture_text or name == "score-truncated-track-boundary.mid"


def test_adversarial_score_generators_are_deterministic_and_parameter_checked() -> None:
    first = score_adversarial_vectors()
    assert first == score_adversarial_vectors()
    assert set(first) == {
        "score-overlap-queue.mid",
        "score-invalid-text-expansion.mid",
        "score-global-density.mid",
        "score-max-vlq.mid",
        "score-system-event-continuation.mid",
        "score-event-density.mid",
        "score-source-tracks.mid",
        "score-missing-eot-policy.mid",
        "score-events-after-eot-policy.mid",
        "score-running-after-sysex-policy.mid",
        "score-running-after-f7-policy.mid",
        "score-running-after-meta-policy.mid",
        "score-valid-f7.mid",
        "score-zero-text-meta.mid",
        "score-format-0.mid",
        "score-format-2.mid",
        "score-zero-division-policy.mid",
        "score-invalid-smpte-division-policy.mid",
    }
    for key in ("overlaps", "invalid_text_bytes", "global_events", "events"):
        with pytest.raises(ValueError):
            score_adversarial_vectors(**{key: 0})
    with pytest.raises(ValueError):
        score_adversarial_vectors(source_tracks=0)
    with pytest.raises(ValueError):
        score_adversarial_vectors(source_tracks=0x1_0000)


def test_current_extension_handles_bounded_adversarial_score_semantics() -> None:
    vectors = score_adversarial_vectors()

    overlap = parse_score(vectors["score-overlap-queue.mid"]).semantic_records()
    notes = overlap["tracks"][0]["notes"]
    assert len(notes) == 1_024
    assert notes[:3] == [
        {"time": 0, "duration": 0, "pitch": 60, "velocity": 1},
        {"time": 0, "duration": 0, "pitch": 60, "velocity": 2},
        {"time": 0, "duration": 0, "pitch": 60, "velocity": 3},
    ]
    assert notes[-1] == {"time": 0, "duration": 0, "pitch": 60, "velocity": 8}

    text = parse_score(vectors["score-invalid-text-expansion.mid"]).semantic_records()
    assert text["tracks"][0]["name"] == "\ufffd" * 128
    assert text["markers"] == [{"time": 1, "text": "\ufffd" * 128}]

    globals_records = parse_score(vectors["score-global-density.mid"]).semantic_records()
    assert len(globals_records["tempos"]) == 64
    assert len(globals_records["time_signatures"]) == 64
    assert len(globals_records["key_signatures"]) == 64
    assert len(globals_records["markers"]) == 64
    assert globals_records["tempos"][0] == {"time": 0, "mspq": 400_000}
    assert globals_records["tempos"][-1] == {"time": 0, "mspq": 400_063}

    max_vlq = parse_score(vectors["score-max-vlq.mid"]).semantic_records()
    assert max_vlq["tracks"][0]["notes"] == [
        {"time": 0, "duration": 0x0FFF_FFFF, "pitch": 60, "velocity": 1}
    ]

    system = parse_score(vectors["score-system-event-continuation.mid"]).semantic_records()
    assert system["tracks"][0]["notes"] == [
        {"time": 0, "duration": 1, "pitch": 60, "velocity": 1}
    ]
    valid_f7 = parse_score(vectors["score-valid-f7.mid"]).semantic_records()
    assert valid_f7["tracks"][0]["notes"] == [
        {"time": 0, "duration": 1, "pitch": 60, "velocity": 1}
    ]
    zero_text = parse_score(vectors["score-zero-text-meta.mid"]).semantic_records()
    assert zero_text["tracks"][0]["name"] == ""
    assert zero_text["markers"] == []
    assert scan(vectors["score-format-0.mid"]).format == 0
    assert scan(vectors["score-format-2.mid"]).format == 2
    assert scan(vectors["score-event-density.mid"]).events == 256
    assert scan(vectors["score-source-tracks.mid"]).tracks == 17


def test_resource_header_vectors_fail_without_materialising_declared_size() -> None:
    for data in score_resource_header_vectors().values():
        with pytest.raises(ValueError, match="SMF parse error at byte 22: unexpected end of input"):
            scan(data)


def test_default_eot_and_division_compatibility_policy_is_explicit() -> None:
    """Default score parsing is finite and stops at EOT; legacy remains opt-in."""
    vectors = score_adversarial_vectors()

    missing_eot = parse_score(vectors["score-missing-eot-policy.mid"]).semantic_records()
    post_eot = parse_score(vectors["score-events-after-eot-policy.mid"]).semantic_records()
    zero_division = parse_score(vectors["score-zero-division-policy.mid"]).semantic_records()
    invalid_smpte = parse_score(
        vectors["score-invalid-smpte-division-policy.mid"]
    ).semantic_records()

    assert missing_eot["tracks"][0]["notes"] == [
        {"time": 0, "duration": 1, "pitch": 60, "velocity": 1}
    ]
    assert post_eot["tracks"] == []
    assert parse_score_unlimited(vectors["score-events-after-eot-policy.mid"]).semantic_records()[
        "tracks"
    ][0]["notes"] == [
        {"time": 0, "duration": 1, "pitch": 60, "velocity": 1}
    ]
    assert zero_division["tpq"] == 0
    assert invalid_smpte["tpq"] == 0x8000


@pytest.mark.parametrize(
    ("name", "data", "offset"),
    [
        (name, data, offset)
        for name, (data, offset) in score_running_status_cancellation_vectors().items()
    ],
)
def test_system_and_meta_events_cancel_channel_running_status(
    name: str, data: bytes, offset: int
) -> None:
    """F0, F7, and FF are not transparent to SMF channel running status."""
    expected = (
        f"SMF parse error at byte {offset}: "
        "data byte encountered without channel running status"
    )
    for parser in (scan, parse, parse_score):
        with pytest.raises(ValueError) as error:
            parser(data)
        assert str(error.value) == expected, name
