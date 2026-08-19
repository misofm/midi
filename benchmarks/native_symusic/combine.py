"""Fail-closed pooling and comparison for interleaved native raw reports."""

from __future__ import annotations

import argparse
from collections.abc import Iterable, Mapping
import json
import math
from pathlib import Path
from typing import Any

from benchmarks.native_symusic.preflight import EXPECTED


RUST_SCHEMA = "miso-native-score-benchmark/v1"
SYMUSIC_SCHEMA = "miso-native-symusic-benchmark/v1"
PREFLIGHT_SCHEMA = "miso-native-score-preflight/v1"
_CONFIG_FIELDS = ("datasets", "samples", "warmup", "iterations", "min_sample_ns", "parse_only", "timed_operation")
_COUNT_FIELDS = tuple(next(iter(EXPECTED.values()))[2].as_dict())
_SYMUSIC_COMMIT = "43ff25277abbc72dbd8d00fb5a9a14ec37fb7906"
MAX_ABBA_MEDIAN_DRIFT_FRACTION = 0.05


def _read(path: Path) -> Mapping[str, Any]:
    value = json.loads(path.read_text())
    if not isinstance(value, Mapping):
        raise ValueError(f"{path}: expected a JSON object")
    return value


def _indexed_datasets(report: Mapping[str, Any], label: str) -> dict[str, Mapping[str, Any]]:
    raw = report.get("datasets")
    if not isinstance(raw, list):
        raise ValueError(f"{label}: datasets must be a list")
    result: dict[str, Mapping[str, Any]] = {}
    for entry in raw:
        if not isinstance(entry, Mapping) or not isinstance(entry.get("dataset"), str):
            raise ValueError(f"{label}: malformed dataset entry")
        name = entry["dataset"]
        if name in result:
            raise ValueError(f"{label}: duplicate dataset {name}")
        result[name] = entry
    return result


def _configuration(report: Mapping[str, Any]) -> dict[str, Any]:
    raw = report.get("configuration")
    if not isinstance(raw, Mapping):
        raise ValueError("native report lacks configuration")
    result = {field: raw.get(field) for field in _CONFIG_FIELDS}
    if any(value is None for value in result.values()):
        raise ValueError("native report has incomplete configuration")
    if result["parse_only"] is not True or result["timed_operation"] != "parse_score_and_destroy":
        raise ValueError("native report is not a parse-only parse-and-destroy measurement")
    if not isinstance(result["samples"], int) or isinstance(result["samples"], bool) or result["samples"] <= 0:
        raise ValueError("native report has invalid configured samples")
    if not isinstance(result["warmup"], int) or isinstance(result["warmup"], bool) or result["warmup"] < 0:
        raise ValueError("native report has invalid configured warmup")
    if result["iterations"] != "auto" and (
        not isinstance(result["iterations"], int)
        or isinstance(result["iterations"], bool)
        or result["iterations"] <= 0
    ):
        raise ValueError("native report has invalid configured iterations")
    if (
        not isinstance(result["min_sample_ns"], int)
        or isinstance(result["min_sample_ns"], bool)
        or result["min_sample_ns"] <= 0
    ):
        raise ValueError("native report has invalid configured minimum sample duration")
    datasets = result["datasets"]
    if (
        not isinstance(datasets, list)
        or not datasets
        or any(not isinstance(name, str) or not name for name in datasets)
        or len(set(datasets)) != len(datasets)
    ):
        raise ValueError("native report has invalid configured datasets")
    return result


def _machine(report: Mapping[str, Any], *, symusic: bool) -> Mapping[str, Any]:
    raw = report.get("machine")
    if not isinstance(raw, Mapping):
        raise ValueError("native report lacks machine metadata")
    if any(raw.get(field) in (None, "unknown") for field in ("cpu_affinity", "cpu_model", "cpu_governor", "kernel_release")):
        raise ValueError("native report has unknown CPU affinity, model, governor, or kernel")
    if raw.get("debug_assertions") is not False:
        raise ValueError("native report was built with debug assertions")
    if symusic:
        if raw.get("build_type") != "Release" or raw.get("ipo_enabled") is not True or raw.get("symusic_library_ipo_enabled") is not True:
            raise ValueError("Symusic report is not an IPO-enabled Release build")
    elif raw.get("cargo_profile") != "release":
        raise ValueError("Miso report is not a Cargo release build")
    elif raw.get("rust_release_profile_config") != {
        "source": "workspace [profile.release]", "lto": "thin", "codegen_units": 1, "panic": "abort"
    }:
        raise ValueError("Miso report does not record the configured workspace release profile")
    return raw


def _counts(entry: Mapping[str, Any]) -> dict[str, Any]:
    raw = entry.get("semantic_contract")
    if not isinstance(raw, Mapping) or raw.get("schema") != "miso-score-contract/v1":
        raise ValueError("native report lacks full semantic-contract metadata")
    summary = raw.get("summary")
    if not isinstance(summary, Mapping) or any(field not in summary for field in _COUNT_FIELDS):
        raise ValueError("native report has incomplete semantic cardinalities")
    return {field: summary[field] for field in _COUNT_FIELDS}


def _samples(entry: Mapping[str, Any], key: str) -> list[float]:
    raw = entry.get(key)
    if not isinstance(raw, Mapping) or not isinstance(raw.get("samples_ns_per_operation"), list):
        raise ValueError(f"native report lacks {key}.samples_ns_per_operation")
    samples = raw["samples_ns_per_operation"]
    if not samples or any(
        isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value <= 0
        for value in samples
    ):
        raise ValueError(f"native report has invalid {key} samples")
    return [float(value) for value in samples]


def _median(values: list[float]) -> float:
    ordered = sorted(values)
    middle = len(ordered) // 2
    return (ordered[middle - 1] + ordered[middle]) / 2 if len(ordered) % 2 == 0 else ordered[middle]


def _geomean(values: Iterable[float]) -> float:
    materialized = list(values)
    if not materialized or any(value <= 0 for value in materialized):
        raise ValueError("geometric mean requires positive values")
    return math.exp(math.fsum(math.log(value) for value in materialized) / len(materialized))


def _validate_preflight(preflight: Mapping[str, Any]) -> Mapping[str, Any]:
    if preflight.get("schema") != PREFLIGHT_SCHEMA or preflight.get("symusic_version") != "0.6.0":
        raise ValueError("unexpected preflight schema or Symusic version")
    datasets = preflight.get("datasets")
    if not isinstance(datasets, Mapping):
        raise ValueError("preflight lacks datasets")
    for name, entry in datasets.items():
        if name not in EXPECTED or not isinstance(entry, Mapping):
            raise ValueError("preflight has an unknown or malformed dataset")
        input_sha, semantic_sha, summary = EXPECTED[name]
        contract = entry.get("semantic_contract")
        if entry.get("input_sha256") != input_sha or not isinstance(contract, Mapping):
            raise ValueError(f"{name}: preflight input hash differs from fixed expectation")
        if contract.get("sha256") != semantic_sha or contract.get("summary") != summary.as_dict():
            raise ValueError(f"{name}: preflight full contract differs from fixed expectation")
    return datasets


def _validate_report(report: Mapping[str, Any], *, symusic: bool, expected_datasets: set[str], config: dict[str, Any] | None, conditions: tuple[str, str, str, str] | None) -> tuple[dict[str, Mapping[str, Any]], dict[str, Any], tuple[str, str, str, str]]:
    schema = SYMUSIC_SCHEMA if symusic else RUST_SCHEMA
    if report.get("schema") != schema:
        raise ValueError(f"unexpected {'Symusic' if symusic else 'Miso'} native report schema")
    if symusic:
        source = report.get("source")
        if not isinstance(source, Mapping) or source.get("commit") != _SYMUSIC_COMMIT:
            raise ValueError("Symusic report is not the exact v0.6.0 source commit")
    report_config = _configuration(report)
    if config is not None and report_config != config:
        raise ValueError("native reports have unequal sample configuration")
    if set(report_config["datasets"]) != expected_datasets:
        raise ValueError("native report configuration and preflight cover different datasets")
    machine = _machine(report, symusic=symusic)
    report_conditions = (
        str(machine["cpu_model"]), str(machine["cpu_affinity"]),
        str(machine["cpu_governor"]), str(machine["kernel_release"]),
    )
    if conditions is not None and report_conditions != conditions:
        raise ValueError("native reports ran with unequal CPU model, affinity, governor, or kernel")
    datasets = _indexed_datasets(report, "Symusic" if symusic else "Miso")
    if set(datasets) != expected_datasets:
        raise ValueError("native reports and preflight cover different datasets")
    return datasets, report_config, report_conditions


def _validate_dataset_timing(
    name: str,
    entries: list[Mapping[str, Any]],
    *,
    distribution_key: str,
    expected_samples: int,
) -> list[float]:
    pooled: list[float] = []
    for entry in entries:
        distribution = entry.get(distribution_key)
        if not isinstance(distribution, Mapping):
            raise ValueError(f"{name}: native report lacks {distribution_key}")
        iterations = distribution.get("iterations")
        if not isinstance(iterations, int) or isinstance(iterations, bool) or iterations <= 0:
            raise ValueError(f"{name}: native report has invalid calibrated iterations")
        samples = _samples(entry, distribution_key)
        if len(samples) != expected_samples:
            raise ValueError(f"{name}: native report sample count differs from configuration")
        pooled.extend(samples)
    return pooled


def _abba_median_drift(name: str, implementation: str, run_samples: list[list[float]]) -> tuple[list[float], float]:
    """Return raw-run medians and reject drift that pooling could conceal."""
    medians = [_median(samples) for samples in run_samples]
    minimum = min(medians)
    drift = max(medians) / minimum - 1.0
    if drift > MAX_ABBA_MEDIAN_DRIFT_FRACTION:
        raise ValueError(
            f"{name}: {implementation} A/B median drift {drift:.3%} exceeds "
            f"the {MAX_ABBA_MEDIAN_DRIFT_FRACTION:.0%} gate"
        )
    return medians, drift


def combine(preflight: Mapping[str, Any], miso_reports: list[Mapping[str, Any]], symusic_reports: list[Mapping[str, Any]]) -> dict[str, Any]:
    """Pool only interleaved reports with identical provenance and conditions."""
    if len(miso_reports) != 2 or len(symusic_reports) != 2:
        raise ValueError("comparison requires exactly two Miso and two Symusic raw reports (ABBA)")
    preflight_datasets = _validate_preflight(preflight)
    expected_names = set(preflight_datasets)
    configuration: dict[str, Any] | None = None
    conditions: tuple[str, str, str, str] | None = None
    validated_miso: list[dict[str, Mapping[str, Any]]] = []
    validated_symusic: list[dict[str, Mapping[str, Any]]] = []
    for report in miso_reports:
        datasets, configuration, conditions = _validate_report(report, symusic=False, expected_datasets=expected_names, config=configuration, conditions=conditions)
        validated_miso.append(datasets)
    for report in symusic_reports:
        datasets, configuration, conditions = _validate_report(report, symusic=True, expected_datasets=expected_names, config=configuration, conditions=conditions)
        validated_symusic.append(datasets)

    results: list[dict[str, Any]] = []
    for name in sorted(expected_names):
        input_sha, semantic_sha, summary = EXPECTED[name]
        expected_summary = summary.as_dict()
        left_entries = [report[name] for report in validated_miso]
        right_entries = [report[name] for report in validated_symusic]
        for entry in [*left_entries, *right_entries]:
            if entry.get("input_sha256") != input_sha or entry.get("input_bytes") != preflight_datasets[name].get("input_bytes"):
                raise ValueError(f"{name}: native input metadata does not match preflight")
            contract = entry.get("semantic_contract")
            if not isinstance(contract, Mapping) or contract.get("sha256") != semantic_sha or _counts(entry) != expected_summary:
                raise ValueError(f"{name}: native full semantic contract does not match fixed expectation")
        assert configuration is not None
        miso_runs = [
            _validate_dataset_timing(
                name, [entry], distribution_key="parse_score_smf", expected_samples=configuration["samples"]
            )
            for entry in left_entries
        ]
        symusic_runs = [
            _validate_dataset_timing(
                name, [entry], distribution_key="parse_score_midi", expected_samples=configuration["samples"]
            )
            for entry in right_entries
        ]
        miso_run_medians, miso_drift = _abba_median_drift(name, "Miso", miso_runs)
        symusic_run_medians, symusic_drift = _abba_median_drift(name, "Symusic", symusic_runs)
        miso_samples = [sample for run in miso_runs for sample in run]
        symusic_samples = [sample for run in symusic_runs for sample in run]
        miso_median = _median(miso_samples)
        symusic_median = _median(symusic_samples)
        results.append({
            "dataset": name,
            "miso_samples_ns_per_operation": miso_samples,
            "symusic_samples_ns_per_operation": symusic_samples,
            "miso_pooled_median_ns": miso_median,
            "symusic_pooled_median_ns": symusic_median,
            "symusic_over_miso_pooled_median_ratio": symusic_median / miso_median,
            "miso_raw_run_medians_ns": miso_run_medians,
            "symusic_raw_run_medians_ns": symusic_run_medians,
            "miso_abba_median_drift_fraction": miso_drift,
            "symusic_abba_median_drift_fraction": symusic_drift,
        })
    return {
        "schema": "miso-native-score-comparison/v3",
        "method": "ABBA interleaved raw reports; pooled only after full contract, condition equality, and per-implementation A/B median-drift gating",
        "abba_median_drift_gate_max_fraction": MAX_ABBA_MEDIAN_DRIFT_FRACTION,
        "configuration": configuration,
        "conditions": {
            "cpu_model": conditions[0], "cpu_affinity": conditions[1],
            "cpu_governor": conditions[2], "kernel_release": conditions[3],
        },
        "datasets": results,
        "geometric_mean_symusic_over_miso_pooled_median_ratio": _geomean(item["symusic_over_miso_pooled_median_ratio"] for item in results),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preflight", type=Path, required=True)
    parser.add_argument("--miso", type=Path, action="append", required=True)
    parser.add_argument("--symusic", type=Path, action="append", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = combine(_read(args.preflight), [_read(path) for path in args.miso], [_read(path) for path in args.symusic])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
