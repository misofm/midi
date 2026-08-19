"""Summarise paired score-contract pyperf results without hiding mismatches."""

from __future__ import annotations

import argparse
from math import exp, fsum, log
from pathlib import Path
from typing import Any

import pyperf


_CONTRACT_PREFIX = "score_"
_REQUIRED_GLOBAL_METADATA = ("score_contract_schema", "miso_midi_version", "symusic_version")


def _load_benchmarks(path: Path) -> dict[str, pyperf.Benchmark]:
    # pyperf 2.x accepts a file object here rather than pathlib.Path.
    with path.open("rb") as source:
        suite = pyperf.BenchmarkSuite.load(source)
    return {benchmark.get_name(): benchmark for benchmark in suite.get_benchmarks()}


def _contract_metadata(metadata: dict[str, Any]) -> dict[str, Any]:
    return {
        key: value
        for key, value in metadata.items()
        if key.startswith(_CONTRACT_PREFIX) or key in _REQUIRED_GLOBAL_METADATA
    }


def _require_same_contract(dataset: str, miso: pyperf.Benchmark, symusic: pyperf.Benchmark) -> None:
    left = _contract_metadata(miso.get_metadata())
    right = _contract_metadata(symusic.get_metadata())
    missing = [key for key in _REQUIRED_GLOBAL_METADATA if key not in left or key not in right]
    if missing:
        raise ValueError(f"{dataset}: missing required contract metadata: {', '.join(missing)}")
    if left != right:
        keys = sorted(set(left) | set(right))
        differences = [f"{key}: {left.get(key)!r} != {right.get(key)!r}" for key in keys if left.get(key) != right.get(key)]
        raise ValueError(f"{dataset}: unequal contract metadata; " + "; ".join(differences))


def summarize(path: Path) -> list[tuple[str, float, float, float, float]]:
    """Return dataset, medians, means, and Symusic/Miso median speedup."""
    benchmarks = _load_benchmarks(path)
    datasets = sorted(
        name.removeprefix("miso/parse-score/")
        for name in benchmarks
        if name.startswith("miso/parse-score/")
    )
    if not datasets:
        raise ValueError("no miso/parse-score/<dataset> benchmarks found")
    symusic_datasets = {
        name.removeprefix("symusic/parse-score/")
        for name in benchmarks
        if name.startswith("symusic/parse-score/")
    }
    if set(datasets) != symusic_datasets:
        raise ValueError(
            "unpaired score benchmarks; "
            f"miso={datasets}, symusic={sorted(symusic_datasets)}"
        )

    rows = []
    for dataset in datasets:
        miso_name = f"miso/parse-score/{dataset}"
        symusic_name = f"symusic/parse-score/{dataset}"
        if symusic_name not in benchmarks:
            raise ValueError(f"{dataset}: missing {symusic_name}")
        miso = benchmarks[miso_name]
        symusic = benchmarks[symusic_name]
        _require_same_contract(dataset, miso, symusic)
        miso_median = miso.median()
        symusic_median = symusic.median()
        rows.append((dataset, miso_median, symusic_median, miso.mean(), symusic.mean()))
    return rows


def summarize_default_overhead(path: Path) -> list[tuple[str, float, float, float, float]]:
    """Return checked/default and trusted/unlimited times when all are present.

    This is a diagnostic comparison of the Python score policy only. It is
    deliberately optional so historical two-series artifacts retain their
    original headline interpretation.
    """
    benchmarks = _load_benchmarks(path)
    datasets = sorted(
        name.removeprefix("miso/parse-score/")
        for name in benchmarks
        if name.startswith("miso/parse-score/")
    )
    unlimited_datasets = {
        name.removeprefix("miso-unlimited/parse-score/")
        for name in benchmarks
        if name.startswith("miso-unlimited/parse-score/")
    }
    if not unlimited_datasets:
        return []
    if set(datasets) != unlimited_datasets:
        raise ValueError(
            "unpaired default/unlimited Miso score benchmarks; "
            f"default={datasets}, unlimited={sorted(unlimited_datasets)}"
        )

    rows = []
    for dataset in datasets:
        checked = benchmarks[f"miso/parse-score/{dataset}"]
        unlimited = benchmarks[f"miso-unlimited/parse-score/{dataset}"]
        _require_same_contract(dataset, checked, unlimited)
        rows.append((dataset, checked.median(), unlimited.median(), checked.mean(), unlimited.mean()))
    return rows


def render_summary(
    rows: list[tuple[str, float, float, float, float]],
    default_overhead_rows: list[tuple[str, float, float, float, float]],
) -> str:
    """Render the checked headline and optional trusted-path diagnostic."""
    lines = ["dataset\tmiso median\tsymusic median\tmedian speedup\tmean speedup"]
    median_speedups = []
    mean_speedups = []
    for dataset, miso_median, symusic_median, miso_mean, symusic_mean in rows:
        median_speedup = symusic_median / miso_median
        mean_speedup = symusic_mean / miso_mean
        median_speedups.append(median_speedup)
        mean_speedups.append(mean_speedup)
        lines.append(
            f"{dataset}\t{miso_median:.9g}s\t{symusic_median:.9g}s\t"
            f"{median_speedup:.3f}x\t{mean_speedup:.3f}x"
        )
    geometric_median = exp(fsum(log(value) for value in median_speedups) / len(median_speedups))
    geometric_mean = exp(fsum(log(value) for value in mean_speedups) / len(mean_speedups))
    lines.append(f"geometric mean\t-\t-\t{geometric_median:.3f}x\t{geometric_mean:.3f}x")

    if default_overhead_rows:
        lines.extend(("", "diagnostic: finite-default policy overhead (default/unlimited)"))
        lines.append("dataset\tdefault median\tunlimited median\tmedian overhead\tmean overhead")
        for dataset, checked_median, unlimited_median, checked_mean, unlimited_mean in default_overhead_rows:
            lines.append(
                f"{dataset}\t{checked_median:.9g}s\t{unlimited_median:.9g}s\t"
                f"{checked_median / unlimited_median:.3f}x\t{checked_mean / unlimited_mean:.3f}x"
            )
    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path, help="pyperf JSON emitted by benchmarks/bench_score.py")
    args = parser.parse_args()
    rows = summarize(args.input)
    print(render_summary(rows, summarize_default_overhead(args.input)))


if __name__ == "__main__":
    main()
