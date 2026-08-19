from __future__ import annotations

from copy import deepcopy
from pathlib import Path

import pytest

from benchmarks.native_symusic.combine import combine
from benchmarks.native_symusic.preflight import EXPECTED


def _reports() -> tuple[dict[object, object], list[dict[object, object]], list[dict[object, object]]]:
    input_sha, semantic_sha, summary = EXPECTED["tiny"]
    expected_summary = summary.as_dict()
    preflight = {
        "schema": "miso-native-score-preflight/v1",
        "symusic_version": "0.6.0",
        "datasets": {"tiny": {"input_bytes": 171, "input_sha256": input_sha, "semantic_contract": {"sha256": semantic_sha, "summary": dict(expected_summary)}}},
    }
    configuration = {
        "datasets": ["tiny"],
        "samples": 2,
        "warmup": 1,
        "iterations": "auto",
        "min_sample_ns": 50,
        "parse_only": True,
        "timed_operation": "parse_score_and_destroy",
    }
    miso = {
        "schema": "miso-native-score-benchmark/v1",
        "configuration": configuration,
        "machine": {"cpu_model": "Test CPU", "cpu_affinity": "4", "cpu_governor": "powersave", "kernel_release": "6.8.0", "debug_assertions": False, "cargo_profile": "release", "rust_release_profile_config": {"source": "workspace [profile.release]", "lto": "thin", "codegen_units": 1, "panic": "abort"}},
        "datasets": [{"dataset": "tiny", "input_bytes": 171, "input_sha256": input_sha, "semantic_contract": {"schema": "miso-score-contract/v1", "sha256": semantic_sha, "summary": dict(expected_summary)}, "parse_score_smf": {"iterations": 3, "samples_ns_per_operation": [10.0, 12.0]}}],
    }
    symusic = {
        "schema": "miso-native-symusic-benchmark/v1",
        "source": {"commit": "43ff25277abbc72dbd8d00fb5a9a14ec37fb7906"},
        "configuration": configuration,
        "machine": {"cpu_model": "Test CPU", "cpu_affinity": "4", "cpu_governor": "powersave", "kernel_release": "6.8.0", "debug_assertions": False, "build_type": "Release", "ipo_enabled": True, "symusic_library_ipo_enabled": True},
        "datasets": [{"dataset": "tiny", "input_bytes": 171, "input_sha256": input_sha, "semantic_contract": {"schema": "miso-score-contract/v1", "sha256": semantic_sha, "summary": dict(expected_summary)}, "parse_score_midi": {"iterations": 3, "samples_ns_per_operation": [20.0, 24.0]}}],
    }
    return preflight, [deepcopy(miso), deepcopy(miso)], [deepcopy(symusic), deepcopy(symusic)]


def test_combine_pools_abba_samples_and_derives_geometric_mean() -> None:
    result = combine(*_reports())
    dataset = result["datasets"][0]
    assert dataset["miso_samples_ns_per_operation"] == [10.0, 12.0, 10.0, 12.0]
    assert dataset["symusic_samples_ns_per_operation"] == [20.0, 24.0, 20.0, 24.0]
    assert dataset["symusic_over_miso_pooled_median_ratio"] == 2.0
    assert dataset["miso_raw_run_medians_ns"] == [11.0, 11.0]
    assert dataset["symusic_raw_run_medians_ns"] == [22.0, 22.0]
    assert dataset["miso_abba_median_drift_fraction"] == 0.0
    assert dataset["symusic_abba_median_drift_fraction"] == 0.0
    assert result["abba_median_drift_gate_max_fraction"] == 0.05
    assert result["geometric_mean_symusic_over_miso_pooled_median_ratio"] == 2.0


@pytest.mark.parametrize(
    ("mutate", "match"),
    [
        (lambda _miso, symusic: symusic[0]["source"].update(commit="wrong"), "exact v0.6.0"),
        (lambda miso, _symusic: miso[0]["machine"].update(cpu_affinity="5"), "unequal CPU model, affinity"),
        (lambda _miso, symusic: symusic[0]["machine"].update(cpu_governor="performance"), "unequal CPU model, affinity"),
        (lambda _miso, symusic: symusic[0]["configuration"].update(samples=3), "unequal sample"),
        (lambda miso, _symusic: miso[0]["datasets"][0]["semantic_contract"].update(sha256="wrong"), "full semantic"),
        (lambda miso, _symusic: miso[0]["datasets"][0]["parse_score_smf"].update(samples_ns_per_operation=[float("nan"), 12.0]), "invalid parse_score_smf"),
        (lambda _miso, symusic: symusic[0]["datasets"][0]["parse_score_midi"].update(samples_ns_per_operation=[20.0, float("inf")]), "invalid parse_score_midi"),
        (lambda _miso, symusic: symusic[0]["datasets"][0]["parse_score_midi"].update(samples_ns_per_operation=[20.0]), "sample count"),
        (lambda miso, _symusic: miso[0]["datasets"][0]["parse_score_smf"].update(iterations=0), "invalid calibrated iterations"),
        (lambda miso, _symusic: miso[0]["configuration"].update(datasets=["tiny", "tiny"]), "configured datasets"),
    ],
)
def test_combine_fails_closed_on_provenance_conditions_or_contract(mutate: object, match: str) -> None:
    preflight, miso, symusic = _reports()
    mutate(miso, symusic)
    with pytest.raises(ValueError, match=match):
        combine(preflight, miso, symusic)


def test_combine_rejects_a_b_median_drift_that_pooling_would_hide() -> None:
    preflight, miso, symusic = _reports()
    miso[1]["datasets"][0]["parse_score_smf"]["samples_ns_per_operation"] = [20.0, 22.0]

    with pytest.raises(ValueError, match=r"Miso A/B median drift .* exceeds the 5% gate"):
        combine(preflight, miso, symusic)


def test_native_source_pin_and_dirty_checkout_guards_are_present() -> None:
    root = Path(__file__).parents[2]
    fetch = (root / "benchmarks/native_symusic/fetch_symusic.sh").read_text()
    cmake = (root / "benchmarks/native_symusic/CMakeLists.txt").read_text()
    expected = "43ff25277abbc72dbd8d00fb5a9a14ec37fb7906"
    assert expected in fetch and expected in cmake
    assert "status --porcelain --untracked-files=all" in fetch
    assert "submodule foreach --recursive" in fetch
    assert 'uv run --project "${repo_root}/benchmarks" cmake -G Ninja' in fetch
    assert "rev-parse HEAD" in cmake
    assert "check_ipo_supported" in cmake
