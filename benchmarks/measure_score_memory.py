"""Measure retained equal-semantics score memory without timing parsing.

The driver verifies the complete canonical Miso/Symusic score contract before
starting any worker.  Every implementation/dataset pair then runs in a new
Linux subprocess, which isolates imports, allocator baselines, and retained
native allocations.  Workers report current RSS at several retained-score
checkpoints; the primary number is the least-squares RSS slope in bytes/score.
"""

from __future__ import annotations

import argparse
from dataclasses import asdict, dataclass
import gc
import hashlib
from importlib.metadata import version
import json
import os
from pathlib import Path
import subprocess
import sys
from typing import Any, Callable, Iterable

MEMORY_SCHEMA = "miso-retained-score-memory/v1"
_RSS_STATM = Path("/proc/self/statm")
_IMPLEMENTATIONS = ("miso", "symusic")
DEFAULT_DATASETS = ("tiny", "normal", "huge", "mahler")


@dataclass(frozen=True)
class RssCheckpoint:
    retained_scores: int
    rss_bytes: int
    rss_delta_from_baseline_bytes: int
    full_retained_bytes_per_score: float


@dataclass(frozen=True)
class WorkerMeasurement:
    implementation: str
    library_version: str
    dataset: str
    input_bytes: int
    input_sha256: str
    retained_scores: int
    checkpoints: list[RssCheckpoint]
    rss_baseline_after_import_and_input_bytes: int
    rss_after_python_handle_list_bytes: int
    python_handle_list_bytes: int
    python_handle_slot_bytes: float
    python_score_proxy_size_bytes: int
    rss_slope_bytes_per_score: float
    final_full_retained_bytes_per_score: float


def require_linux_current_rss() -> None:
    """Fail clearly instead of silently substituting a different memory metric."""
    if sys.platform != "linux" or not _RSS_STATM.is_file():
        raise RuntimeError(
            "retained-score memory measurement requires Linux /proc/self/statm "
            "for current RSS; this harness intentionally has no cross-platform fallback"
        )


def current_rss_bytes(statm_path: Path = _RSS_STATM, page_size: int | None = None) -> int:
    """Read Linux current resident set size, not max RSS or a sampled peak."""
    fields = statm_path.read_text().split()
    if len(fields) < 2:
        raise RuntimeError(f"invalid Linux statm data in {statm_path}")
    try:
        resident_pages = int(fields[1])
    except ValueError as error:
        raise RuntimeError(f"invalid resident-page count in {statm_path}: {fields[1]!r}") from error
    if resident_pages < 0:
        raise RuntimeError(f"negative resident-page count in {statm_path}")
    if page_size is None:
        page_size = os.sysconf("SC_PAGE_SIZE")
    return resident_pages * page_size


def checkpoint_counts(count: int, requested: str | None = None) -> list[int]:
    """Return at least two strictly increasing retained-score checkpoints."""
    if count < 2:
        raise ValueError("--count must be at least 2 so an RSS slope is defined")
    if requested:
        try:
            values = [int(value) for value in requested.split(",")]
        except ValueError as error:
            raise ValueError("--checkpoints must be comma-separated positive integers") from error
        if not values or any(value <= 0 for value in values):
            raise ValueError("--checkpoints must contain positive integers")
        if values != sorted(set(values)):
            raise ValueError("--checkpoints must be strictly increasing with no duplicates")
        if values[-1] != count:
            raise ValueError("--checkpoints must end with --count so final bytes/score is reported")
        if values[-1] > count:
            raise ValueError("--checkpoints cannot exceed --count")
        if len(values) < 2:
            raise ValueError("--checkpoints must contain at least two values")
        return values

    values = [1]
    while values[-1] * 2 < count:
        values.append(values[-1] * 2)
    values.append(count)
    return values


def rss_slope_bytes_per_score(checkpoints: Iterable[RssCheckpoint]) -> float:
    """Least-squares slope over raw current-RSS checkpoints."""
    points = list(checkpoints)
    if len(points) < 2:
        raise ValueError("at least two RSS checkpoints are required")
    mean_count = sum(point.retained_scores for point in points) / len(points)
    mean_rss = sum(point.rss_bytes for point in points) / len(points)
    denominator = sum((point.retained_scores - mean_count) ** 2 for point in points)
    if denominator == 0:
        raise ValueError("RSS checkpoints must use distinct retained-score counts")
    numerator = sum(
        (point.retained_scores - mean_count) * (point.rss_bytes - mean_rss)
        for point in points
    )
    return numerator / denominator


def _resolve_score_parser(implementation: str) -> tuple[Callable[[bytes], object], str]:
    """Import only the selected library in a fresh worker process."""
    if implementation == "miso":
        import miso_midi

        return miso_midi.parse_score, version("miso-midi")
    if implementation == "symusic":
        from symusic import Score

        return Score.from_midi, version("symusic")
    raise ValueError(f"unsupported implementation: {implementation!r}")


def collect_retained_scores(
    *,
    implementation: str,
    library_version: str,
    dataset: str,
    data: bytes,
    count: int,
    checkpoints: list[int],
    parse_score: Callable[[bytes], object],
    rss_reader: Callable[[], int] = current_rss_bytes,
) -> WorkerMeasurement:
    """Retain score objects and collect RSS; deliberately contains no timers."""
    if checkpoints != sorted(set(checkpoints)) or checkpoints[-1] != count:
        raise ValueError("checkpoints must be strictly increasing and end at count")

    # The data buffer is already resident and the selected library imported.
    # GC removes incidental import/setup garbage before the isolated baseline.
    gc.collect()
    baseline = rss_reader()

    # Preallocate the exact Python list that owns score handles.  Its RSS and
    # ``sys.getsizeof`` are both reported, rather than being hidden from the
    # retained-memory number as an implementation detail.
    scores: list[object | None] = [None] * count
    handle_list_bytes = sys.getsizeof(scores)
    empty_list_bytes = sys.getsizeof([])
    handle_slot_bytes = (handle_list_bytes - empty_list_bytes) / count
    after_handle_list = rss_reader()

    checkpoint_set = set(checkpoints)
    raw_checkpoints: list[RssCheckpoint] = []
    proxy_size = 0
    for index in range(count):
        score = parse_score(data)
        scores[index] = score
        if index == 0:
            proxy_size = sys.getsizeof(score)
        retained = index + 1
        if retained in checkpoint_set:
            rss = rss_reader()
            raw_checkpoints.append(
                RssCheckpoint(
                    retained_scores=retained,
                    rss_bytes=rss,
                    rss_delta_from_baseline_bytes=rss - baseline,
                    full_retained_bytes_per_score=(rss - baseline) / retained,
                )
            )

    slope = rss_slope_bytes_per_score(raw_checkpoints)
    return WorkerMeasurement(
        implementation=implementation,
        library_version=library_version,
        dataset=dataset,
        input_bytes=len(data),
        input_sha256=hashlib.sha256(data).hexdigest(),
        retained_scores=count,
        checkpoints=raw_checkpoints,
        rss_baseline_after_import_and_input_bytes=baseline,
        rss_after_python_handle_list_bytes=after_handle_list,
        python_handle_list_bytes=handle_list_bytes,
        python_handle_slot_bytes=handle_slot_bytes,
        python_score_proxy_size_bytes=proxy_size,
        rss_slope_bytes_per_score=slope,
        final_full_retained_bytes_per_score=raw_checkpoints[-1].full_retained_bytes_per_score,
    )


def run_worker(
    implementation: str,
    dataset: str,
    data_path: Path,
    count: int,
    checkpoints: list[int],
) -> WorkerMeasurement:
    require_linux_current_rss()
    # Hold input bytes before the post-import baseline so the benchmark measures
    # retained score objects, not an implementation-independent input buffer.
    data = data_path.read_bytes()
    parse_score, library_version = _resolve_score_parser(implementation)
    return collect_retained_scores(
        implementation=implementation,
        library_version=library_version,
        dataset=dataset,
        data=data,
        count=count,
        checkpoints=checkpoints,
        parse_score=parse_score,
    )


def build_worker_command(
    script: Path,
    implementation: str,
    dataset: str,
    data_path: Path,
    count: int,
    checkpoints: list[int],
) -> list[str]:
    return [
        sys.executable,
        str(script),
        "--worker",
        "--implementation",
        implementation,
        "--dataset",
        dataset,
        "--data-path",
        str(data_path),
        "--count",
        str(count),
        "--checkpoints",
        ",".join(str(value) for value in checkpoints),
    ]


def run_isolated_worker(command: list[str]) -> WorkerMeasurement:
    """Run and decode one clean-process worker, surfacing its stderr on error."""
    completed = subprocess.run(command, check=False, capture_output=True, text=True)
    if completed.returncode:
        raise RuntimeError(
            f"retained-memory worker failed with exit code {completed.returncode}:\n"
            f"command: {' '.join(command)}\n{completed.stderr.strip()}"
        )
    try:
        value = json.loads(completed.stdout)
        checkpoints = [RssCheckpoint(**point) for point in value.pop("checkpoints")]
        return WorkerMeasurement(checkpoints=checkpoints, **value)
    except (TypeError, ValueError, json.JSONDecodeError) as error:
        raise RuntimeError(
            f"retained-memory worker returned invalid JSON: {completed.stdout!r}"
        ) from error


def build_report(corpus: Path, datasets: list[str], count: int, checkpoints: list[int]) -> dict[str, Any]:
    """Preflight canonical equality, then collect isolated worker measurements."""
    require_linux_current_rss()
    # This import is intentionally parent-only.  Workers must not import the
    # competing library before recording their selected-library baseline.
    try:
        from benchmarks.bench_score import _load_and_verify
    except ModuleNotFoundError:  # pragma: no cover - direct-script import path
        from bench_score import _load_and_verify  # type: ignore[no-redef]

    prepared = _load_and_verify(corpus, datasets)
    script = Path(__file__).resolve()
    dataset_reports: dict[str, Any] = {}
    for dataset, data, contract in prepared:
        workers = {}
        for implementation in _IMPLEMENTATIONS:
            command = build_worker_command(
                script, implementation, dataset, corpus / f"{dataset}.mid", count, checkpoints
            )
            workers[implementation] = asdict(run_isolated_worker(command))
        dataset_reports[dataset] = {
            "input_bytes": len(data),
            "input_sha256": hashlib.sha256(data).hexdigest(),
            "semantic_contract_sha256": contract.sha256,
            "semantic_summary": contract.summary.as_dict(),
            "workers": workers,
        }
    return {
        "schema": MEMORY_SCHEMA,
        "platform": sys.platform,
        "rss_source": "/proc/self/statm resident pages * SC_PAGE_SIZE",
        "retained_scores_per_checkpoint_final": count,
        "checkpoint_counts": checkpoints,
        "datasets": dataset_reports,
    }


def render_report(report: dict[str, Any]) -> str:
    """Render raw checkpoints alongside slope and inclusive bytes/score."""
    lines = [
        f"{report['schema']} ({report['rss_source']})",
        "Values are retained-memory measurements; no parse timing is included.",
    ]
    for dataset, dataset_report in report["datasets"].items():
        lines.append(
            f"{dataset}: input={dataset_report['input_bytes']}B "
            f"contract={dataset_report['semantic_contract_sha256']}"
        )
        for implementation, worker in dataset_report["workers"].items():
            lines.append(
                f"  {implementation} {worker['library_version']}: "
                f"RSS slope={worker['rss_slope_bytes_per_score']:.2f} B/score; "
                f"final inclusive={worker['final_full_retained_bytes_per_score']:.2f} B/score"
            )
            lines.append(
                f"    post-import/input RSS={worker['rss_baseline_after_import_and_input_bytes']} B; "
                f"post-list RSS={worker['rss_after_python_handle_list_bytes']} B; "
                f"Python handle-list={worker['python_handle_list_bytes']} B "
                f"({worker['python_handle_slot_bytes']:.2f} B/slot); "
                f"first score proxy={worker['python_score_proxy_size_bytes']} B"
            )
            for point in worker["checkpoints"]:
                lines.append(
                    f"    retained={point['retained_scores']}: RSS={point['rss_bytes']} B, "
                    f"delta={point['rss_delta_from_baseline_bytes']} B, "
                    f"inclusive={point['full_retained_bytes_per_score']:.2f} B/score"
                )
    return "\n".join(lines)


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--implementation", choices=_IMPLEMENTATIONS)
    parser.add_argument("--dataset", choices=DEFAULT_DATASETS)
    parser.add_argument("--data-path", type=Path)
    parser.add_argument("--corpus", type=Path, default=Path(__file__).with_name("corpus"))
    parser.add_argument("--datasets", nargs="+", choices=DEFAULT_DATASETS, default=list(DEFAULT_DATASETS))
    parser.add_argument("--count", type=int, default=64, help="retained scores per implementation/dataset")
    parser.add_argument(
        "--checkpoints",
        help="strictly increasing comma-separated retained counts ending at --count",
    )
    parser.add_argument("--output", type=Path, help="write the complete JSON report to this path")
    return parser


def main() -> None:
    parser = _parser()
    args = parser.parse_args()
    try:
        checkpoints = checkpoint_counts(args.count, args.checkpoints)
        if args.worker:
            if not args.implementation or not args.dataset or not args.data_path:
                parser.error("--worker requires --implementation, --dataset, and --data-path")
            print(
                json.dumps(
                    asdict(run_worker(args.implementation, args.dataset, args.data_path, args.count, checkpoints)),
                    sort_keys=True,
                )
            )
            return
        report = build_report(args.corpus, args.datasets, args.count, checkpoints)
    except (OSError, RuntimeError, ValueError) as error:
        parser.error(str(error))

    encoded = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(encoded)
        print(render_report(report))
    else:
        print(encoded, end="")


if __name__ == "__main__":
    main()
