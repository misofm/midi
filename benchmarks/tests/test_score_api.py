from __future__ import annotations

import hashlib
from pathlib import Path

import pytest
from symusic import Score as SymusicScore

from benchmarks.corpus import REFERENCE_SHA256, make_file, malformed_files
from benchmarks.score_contract import score_contract_from_records, symusic_score_contract
from miso_midi import Score, parse_score


def _assert_equal_contract(data: bytes) -> None:
    ours = parse_score(data)
    theirs = symusic_score_contract(SymusicScore.from_midi(data))
    actual = score_contract_from_records(ours.semantic_records())

    # Keep digest and every cardinality separate in the failure output. A
    # matching note count alone is not enough for an equal-score benchmark.
    assert actual.sha256 == theirs.sha256
    assert actual.summary.as_dict() == theirs.summary.as_dict()

    summary = actual.summary
    assert ours.track_count == summary.tracks
    assert ours.note_count == summary.notes
    assert ours.control_count == summary.controls
    assert ours.pitch_bend_count == summary.pitch_bends
    assert ours.pedal_count == summary.pedals
    assert ours.lyric_count == summary.lyrics
    assert ours.time_signature_count == summary.time_signatures
    assert ours.key_signature_count == summary.key_signatures
    assert ours.tempo_count == summary.tempos
    assert ours.marker_count == summary.markers
    assert ours.ticks_per_quarter == 480
    assert ours.bytes_consumed == len(data)
    assert ours.trailing_bytes == 0


@pytest.mark.parametrize(("tracks", "notes"), [(1, 16), (8, 2_000)])
def test_generated_scores_match_complete_symusic_contract(tracks: int, notes: int) -> None:
    data, _ = make_file(tracks, notes)
    _assert_equal_contract(data)


@pytest.mark.skipif(
    not Path("benchmarks/corpus/mahler.mid").exists(),
    reason="reference corpus has not been generated",
)
def test_checksum_pinned_mahler_matches_complete_symusic_contract() -> None:
    data = Path("benchmarks/corpus/mahler.mid").read_bytes()
    assert hashlib.sha256(data).hexdigest() == REFERENCE_SHA256
    _assert_equal_contract(data)


def test_score_public_api_is_native_and_bulk_materializes_records() -> None:
    data, _ = make_file(1, 16)
    score = parse_score(data)

    assert isinstance(score, Score)
    heap_before = score.heap_bytes
    assert score.note_count == 16
    assert Score(data).semantic_records() == score.semantic_records()
    assert len(score) == score.track_count
    assert "Score(tpq=480, tracks=1, notes=16)" == repr(score)
    assert score.heap_bytes == heap_before
    assert score.heap_bytes > 0
    assert set(score.semantic_records()) == {
        "tpq",
        "tracks",
        "time_signatures",
        "key_signatures",
        "tempos",
        "markers",
    }


@pytest.mark.parametrize("data", malformed_files().values(), ids=malformed_files().keys())
def test_score_parser_rejects_malformed_smf(data: bytes) -> None:
    with pytest.raises(ValueError, match="SMF parse error"):
        parse_score(data)
