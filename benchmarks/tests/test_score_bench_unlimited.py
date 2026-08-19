"""Tests for the optional trusted-path score-benchmark diagnostic."""

from __future__ import annotations

from argparse import Namespace
from pathlib import Path

import pytest

from benchmarks import bench_score, summarize_score
from benchmarks.score_contract import ScoreContract, ScoreSummary


def _contract(digest: str = "a" * 64) -> ScoreContract:
    return ScoreContract(
        canonical=b"",
        sha256=digest,
        summary=ScoreSummary(
            tracks=1,
            notes=1,
            controls=0,
            pitch_bends=0,
            pedals=0,
            lyrics=0,
            time_signatures=0,
            key_signatures=0,
            tempos=0,
            markers=0,
        ),
    )


def test_score_worker_argument_forwarding_is_opt_in() -> None:
    base = Namespace(corpus=Path("corpus"), datasets=["tiny"], include_miso_unlimited=False)
    command: list[str] = []
    bench_score._forward_score_arguments(command, base)
    assert command == ["--corpus", "corpus", "--datasets", "tiny"]

    diagnostic = Namespace(corpus=Path("corpus"), datasets=["tiny"], include_miso_unlimited=True)
    command = []
    bench_score._forward_score_arguments(command, diagnostic)
    assert command == ["--corpus", "corpus", "--datasets", "tiny", "--include-miso-unlimited"]


def test_unlimited_preflight_fails_closed_on_semantic_mismatch(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    (tmp_path / "tiny.mid").write_bytes(b"fixture")
    expected = _contract()
    monkeypatch.setattr(bench_score, "parse_symusic_score", lambda _data: object())
    monkeypatch.setattr(bench_score, "symusic_score_contract", lambda _score: expected)
    monkeypatch.setattr(bench_score, "miso_score_contract", lambda _data: expected)
    monkeypatch.setattr(bench_score, "_miso_unlimited_score_contract", lambda _data: _contract("b" * 64))

    with pytest.raises(RuntimeError, match="miso-unlimited digest"):
        bench_score._load_and_verify(tmp_path, ["tiny"], include_miso_unlimited=True)


class _FakeBenchmark:
    def __init__(self, metadata: dict[str, object], median: float, mean: float) -> None:
        self._metadata = metadata
        self._median = median
        self._mean = mean

    def get_metadata(self) -> dict[str, object]:
        return self._metadata

    def median(self) -> float:
        return self._median

    def mean(self) -> float:
        return self._mean


def _metadata() -> dict[str, object]:
    return {
        "score_contract_schema": "miso-score-contract/v1",
        "miso_midi_version": "0.1.0",
        "symusic_version": "0.6.0",
        "score_tiny_corpus_sha256": "a" * 64,
        "score_tiny_input_bytes": 1,
        "score_tiny_semantic_sha256": "b" * 64,
        "score_tiny_output_tracks": 1,
    }


def test_old_two_series_artifact_keeps_headline_and_has_no_policy_diagnostic(
    monkeypatch: pytest.MonkeyPatch
) -> None:
    metadata = _metadata()
    suite = {
        "miso/parse-score/tiny": _FakeBenchmark(metadata, 1.0, 1.2),
        "symusic/parse-score/tiny": _FakeBenchmark(metadata, 2.0, 2.4),
    }
    monkeypatch.setattr(summarize_score, "_load_benchmarks", lambda _path: suite)

    rows = summarize_score.summarize(Path("old.json"))
    diagnostics = summarize_score.summarize_default_overhead(Path("old.json"))
    output = summarize_score.render_summary(rows, diagnostics)

    assert rows == [("tiny", 1.0, 2.0, 1.2, 2.4)]
    assert diagnostics == []
    assert output == (
        "dataset\tmiso median\tsymusic median\tmedian speedup\tmean speedup\n"
        "tiny\t1s\t2s\t2.000x\t2.000x\n"
        "geometric mean\t-\t-\t2.000x\t2.000x"
    )


def test_three_series_summary_adds_checked_over_unlimited_diagnostic_and_refuses_mismatch(
    monkeypatch: pytest.MonkeyPatch
) -> None:
    metadata = _metadata()
    suite = {
        "miso/parse-score/tiny": _FakeBenchmark(metadata, 1.2, 1.5),
        "miso-unlimited/parse-score/tiny": _FakeBenchmark(metadata, 1.0, 1.2),
        "symusic/parse-score/tiny": _FakeBenchmark(metadata, 2.4, 3.0),
    }
    monkeypatch.setattr(summarize_score, "_load_benchmarks", lambda _path: suite)

    rows = summarize_score.summarize(Path("three.json"))
    diagnostics = summarize_score.summarize_default_overhead(Path("three.json"))
    output = summarize_score.render_summary(rows, diagnostics)

    assert diagnostics == [("tiny", 1.2, 1.0, 1.5, 1.2)]
    assert "geometric mean\t-\t-\t2.000x\t2.000x" in output
    assert "diagnostic: finite-default policy overhead (default/unlimited)" in output
    assert "tiny\t1.2s\t1s\t1.200x\t1.250x" in output

    bad_metadata = dict(metadata, score_tiny_semantic_sha256="c" * 64)
    suite["miso-unlimited/parse-score/tiny"] = _FakeBenchmark(bad_metadata, 1.0, 1.2)
    with pytest.raises(ValueError, match="unequal contract metadata"):
        summarize_score.summarize_default_overhead(Path("three.json"))
