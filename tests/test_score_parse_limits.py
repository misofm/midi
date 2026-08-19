"""Finite-by-default Python score parser policy and public error contract."""

from __future__ import annotations

import pytest

from benchmarks.corpus import make_file, score_adversarial_vectors, score_policy_vectors
from miso_midi import Score, ScoreParseLimits, parse_score, parse_score_unlimited


def test_score_parse_limits_defaults_are_finite_read_only_and_repr_stable() -> None:
    limits = ScoreParseLimits()

    assert (
        limits.max_input_bytes,
        limits.max_source_tracks,
        limits.max_track_bytes,
        limits.max_events,
        limits.max_note_starts,
        limits.max_text_bytes,
    ) == (64 * 1024 * 1024, 4_096, 16 * 1024 * 1024, 2_000_000, 1_000_000, 16 * 1024 * 1024)
    assert repr(limits) == (
        "ScoreParseLimits(max_input_bytes=67108864, max_source_tracks=4096, "
        "max_track_bytes=16777216, max_events=2000000, max_note_starts=1000000, "
        "max_text_bytes=16777216)"
    )
    with pytest.raises(AttributeError):
        limits.max_events = 1  # type: ignore[misc]


@pytest.mark.parametrize(
    ("fixture", "limits", "expected"),
    [
        (
            "score-event-density.mid",
            ScoreParseLimits(max_input_bytes=1),
            "SMF parse error at byte 0: score parse limit exceeded: input bytes (limit 1)",
        ),
        (
            "score-event-density.mid",
            ScoreParseLimits(max_events=1),
            "SMF parse error at byte 27: score parse limit exceeded: events (limit 1)",
        ),
        (
            "score-overlap-queue.mid",
            ScoreParseLimits(max_note_starts=0),
            "SMF parse error at byte 23: score parse limit exceeded: note starts (limit 0)",
        ),
        (
            "score-invalid-text-expansion.mid",
            ScoreParseLimits(max_text_bytes=3),
            "SMF parse error at byte 23: score parse limit exceeded: normalized text bytes (limit 3)",
        ),
    ],
)
def test_custom_score_limits_preserve_exact_value_error_text_and_offset(
    fixture: str, limits: ScoreParseLimits, expected: str
) -> None:
    with pytest.raises(ValueError) as error:
        parse_score(score_adversarial_vectors()[fixture], limits=limits)
    assert str(error.value) == expected


def test_default_and_custom_normal_limits_have_identical_score_and_constructor_path() -> None:
    data, _ = make_file(1, 16)
    custom = ScoreParseLimits(
        max_input_bytes=len(data),
        max_source_tracks=1,
        max_track_bytes=len(data),
        max_events=128,
        max_note_starts=32,
        max_text_bytes=256,
    )

    default = parse_score(data)
    bounded = parse_score(data, limits=custom)
    constructed = Score(data, limits=custom)

    assert default.semantic_records() == bounded.semantic_records() == constructed.semantic_records()


def test_compatible_stops_after_eot_but_unlimited_retains_legacy_post_eot_events() -> None:
    data = score_adversarial_vectors()["score-events-after-eot-policy.mid"]

    assert parse_score(data).semantic_records()["tracks"] == []
    assert parse_score(data, limits=None, mode="compatible").semantic_records()["tracks"] == []
    assert parse_score_unlimited(data).semantic_records()["tracks"][0]["notes"] == [
        {"time": 0, "duration": 1, "pitch": 60, "velocity": 1}
    ]


@pytest.mark.parametrize(
    ("fixture", "expected"),
    [
        (
            "score-missing-eot-policy.mid",
            "SMF parse error at byte 30: declared track ends without End-of-Track",
        ),
        (
            "score-events-after-eot-policy.mid",
            "SMF parse error at byte 26: bytes follow End-of-Track in declared track",
        ),
        (
            "score-zero-division-policy.mid",
            "SMF parse error at byte 12: metrical division has zero ticks per quarter",
        ),
        (
            "score-invalid-smpte-division-policy.mid",
            "SMF parse error at byte 12: invalid SMPTE division: -128 frames per second, 0 ticks per frame",
        ),
    ],
)
def test_strict_mode_exposes_core_eot_and_division_errors(fixture: str, expected: str) -> None:
    with pytest.raises(ValueError) as error:
        parse_score(score_adversarial_vectors()[fixture], mode="strict")
    assert str(error.value) == expected


def test_strict_mode_accepts_valid_smpte_division() -> None:
    data = score_policy_vectors()["score-smpte-division.mid"]
    assert parse_score(data, mode="strict").ticks_per_quarter == 0xE728


def test_strict_mode_rejects_file_trailing_bytes_at_the_consumed_offset() -> None:
    data, _ = make_file(1, 1)

    with pytest.raises(ValueError) as error:
        parse_score(data + b"tail", mode="strict")
    assert str(error.value) == (
        f"SMF parse error at byte {len(data)}: bytes follow declared SMF tracks"
    )


def test_invalid_mode_limits_type_and_integer_ranges_fail_at_the_python_boundary() -> None:
    data, _ = make_file(1, 1)

    with pytest.raises(ValueError, match="mode must be 'compatible' or 'strict'"):
        parse_score(data, mode="legacy")
    with pytest.raises(TypeError):
        parse_score(data, mode=1)  # type: ignore[arg-type]
    with pytest.raises(TypeError, match="ScoreParseLimits"):
        parse_score(data, limits=object())  # type: ignore[arg-type]
    with pytest.raises(OverflowError):
        ScoreParseLimits(max_source_tracks=65_536)
    with pytest.raises(OverflowError):
        ScoreParseLimits(max_events=-1)
