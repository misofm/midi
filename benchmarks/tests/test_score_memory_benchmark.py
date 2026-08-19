from __future__ import annotations

from pathlib import Path
import sys

import pytest

from benchmarks.measure_score_memory import (
    MEMORY_SCHEMA,
    RssCheckpoint,
    build_worker_command,
    checkpoint_counts,
    collect_retained_scores,
    current_rss_bytes,
    render_report,
    require_linux_current_rss,
    rss_slope_bytes_per_score,
)


def test_checkpoint_counts_are_reproducible_and_validate_final_count() -> None:
    assert checkpoint_counts(64) == [1, 2, 4, 8, 16, 32, 64]
    assert checkpoint_counts(96, "1,2,8,32,96") == [1, 2, 8, 32, 96]
    with pytest.raises(ValueError, match="end with --count"):
        checkpoint_counts(64, "1,2,4")
    with pytest.raises(ValueError, match="at least 2"):
        checkpoint_counts(1)


def test_current_rss_uses_resident_statm_pages(tmp_path: Path) -> None:
    statm = tmp_path / "statm"
    statm.write_text("100 23 0 0 0 0 0\n")
    assert current_rss_bytes(statm, page_size=4096) == 23 * 4096


def test_collect_retained_scores_reports_raw_rss_slope_and_handle_list() -> None:
    readings = iter((1_000, 1_024, 2_000, 3_000))
    parsed = []

    def parser(data: bytes) -> object:
        parsed.append(data)
        return object()

    measurement = collect_retained_scores(
        implementation="fake",
        library_version="0",
        dataset="tiny",
        data=b"abc",
        count=2,
        checkpoints=[1, 2],
        parse_score=parser,
        rss_reader=lambda: next(readings),
    )

    assert parsed == [b"abc", b"abc"]
    assert measurement.rss_baseline_after_import_and_input_bytes == 1_000
    assert measurement.rss_after_python_handle_list_bytes == 1_024
    assert measurement.python_handle_list_bytes >= sys.getsizeof([])
    assert measurement.rss_slope_bytes_per_score == 1_000
    assert measurement.final_full_retained_bytes_per_score == 1_000
    assert measurement.checkpoints == [
        RssCheckpoint(1, 2_000, 1_000, 1_000),
        RssCheckpoint(2, 3_000, 2_000, 1_000),
    ]


def test_worker_command_contains_every_reproducibility_input(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("benchmarks.measure_score_memory.sys.executable", "/test/python")
    command = build_worker_command(
        Path("/repo/benchmarks/measure_score_memory.py"),
        "miso",
        "normal",
        Path("/repo/corpus/normal.mid"),
        64,
        [1, 2, 64],
    )

    assert command == [
        "/test/python",
        "/repo/benchmarks/measure_score_memory.py",
        "--worker",
        "--implementation",
        "miso",
        "--dataset",
        "normal",
        "--data-path",
        "/repo/corpus/normal.mid",
        "--count",
        "64",
        "--checkpoints",
        "1,2,64",
    ]


def test_render_report_includes_raw_checkpoints_and_python_handles() -> None:
    worker = {
        "library_version": "0",
        "rss_slope_bytes_per_score": 128.0,
        "final_full_retained_bytes_per_score": 160.0,
        "rss_baseline_after_import_and_input_bytes": 1_000,
        "rss_after_python_handle_list_bytes": 1_064,
        "python_handle_list_bytes": 64,
        "python_handle_slot_bytes": 8.0,
        "python_score_proxy_size_bytes": 40,
        "checkpoints": [
            {
                "retained_scores": 1,
                "rss_bytes": 1_128,
                "rss_delta_from_baseline_bytes": 128,
                "full_retained_bytes_per_score": 128.0,
            },
            {
                "retained_scores": 2,
                "rss_bytes": 1_320,
                "rss_delta_from_baseline_bytes": 320,
                "full_retained_bytes_per_score": 160.0,
            },
        ],
    }
    report = {
        "schema": MEMORY_SCHEMA,
        "rss_source": "mock-current-rss",
        "datasets": {
            "tiny": {
                "input_bytes": 3,
                "semantic_contract_sha256": "a" * 64,
                "workers": {"miso": worker, "symusic": worker},
            }
        },
    }

    rendered = render_report(report)
    assert "RSS slope=128.00 B/score" in rendered
    assert "Python handle-list=64 B" in rendered
    assert "retained=2: RSS=1320 B" in rendered


def test_linux_requirement_fails_clearly_off_linux(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("benchmarks.measure_score_memory.sys.platform", "darwin")
    with pytest.raises(RuntimeError, match="requires Linux /proc/self/statm"):
        require_linux_current_rss()


def test_rss_slope_rejects_duplicate_counts() -> None:
    with pytest.raises(ValueError, match="distinct"):
        rss_slope_bytes_per_score(
            [
                RssCheckpoint(1, 100, 0, 0.0),
                RssCheckpoint(1, 200, 100, 100.0),
            ]
        )
