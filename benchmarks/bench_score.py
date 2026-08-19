"""Equal-semantics in-memory score parsing benchmark.

This command validates the v1 score contract for every selected dataset before
asking pyperf to time either implementation. A semantic mismatch aborts before
the first result is emitted, preventing a partial result from looking like a
competitive benchmark.
"""

from __future__ import annotations

import hashlib
from importlib.metadata import version
from pathlib import Path

import pyperf
from symusic import Score

try:  # Support both ``python benchmarks/bench_score.py`` and module execution.
    from benchmarks.score_contract import (
        MisoScoreApiUnavailable,
        SCORE_CONTRACT_SCHEMA,
        ScoreContract,
        miso_score_contract,
        score_contract_from_records,
        symusic_score_contract,
    )
except ModuleNotFoundError:  # pragma: no cover - direct-script import path
    from score_contract import (  # type: ignore[no-redef]
        MisoScoreApiUnavailable,
        SCORE_CONTRACT_SCHEMA,
        ScoreContract,
        miso_score_contract,
        score_contract_from_records,
        symusic_score_contract,
    )


DEFAULT_DATASETS = ("tiny", "normal", "huge", "mahler")


def parse_symusic_score(data: bytes) -> Score:
    """The timed operation: warm, in-memory byte parsing only."""
    return Score.from_midi(data)


def _same_contract(dataset: str, implementation: str, miso: ScoreContract, symusic: ScoreContract) -> None:
    if miso.sha256 != symusic.sha256 or miso.summary != symusic.summary:
        raise RuntimeError(
            f"semantic mismatch for {dataset}: "
            f"{implementation} digest={miso.sha256} summary={miso.summary.as_dict()}; "
            f"symusic digest={symusic.sha256} summary={symusic.summary.as_dict()}"
        )


def _set_metadata(runner: pyperf.Runner, dataset: str, data: bytes, contract: ScoreContract) -> None:
    # pyperf metadata is copied into every emitted benchmark.  Store only
    # scalar, reproducible values, including output cardinalities.
    runner.metadata["score_contract_schema"] = SCORE_CONTRACT_SCHEMA
    runner.metadata["miso_midi_version"] = version("miso-midi")
    runner.metadata["symusic_version"] = version("symusic")
    runner.metadata[f"score_{dataset}_corpus_sha256"] = hashlib.sha256(data).hexdigest()
    runner.metadata[f"score_{dataset}_input_bytes"] = len(data)
    runner.metadata[f"score_{dataset}_semantic_sha256"] = contract.sha256
    for field, count in contract.summary.as_dict().items():
        runner.metadata[f"score_{dataset}_output_{field}"] = count


def _miso_unlimited_score_contract(data: bytes) -> ScoreContract:
    """Get a bulk canonical contract through the explicit trusted API."""
    import miso_midi

    parser = getattr(miso_midi, "parse_score_unlimited", None)
    if not callable(parser):
        raise MisoScoreApiUnavailable(
            "miso_midi.parse_score_unlimited(bytes) is not available; install a binding "
            "with the explicit trusted score API before using --include-miso-unlimited."
        )
    records = getattr(parser(data), "semantic_records", None)
    if not callable(records):
        raise MisoScoreApiUnavailable(
            "miso_midi.parse_score_unlimited(bytes) must expose Score.semantic_records() "
            "for an equal-semantics diagnostic benchmark."
        )
    return score_contract_from_records(records())


def _load_and_verify(
    corpus: Path, datasets: list[str], include_miso_unlimited: bool = False
) -> list[tuple[str, bytes, ScoreContract]]:
    prepared: list[tuple[str, bytes, ScoreContract]] = []
    for dataset in datasets:
        path = corpus / f"{dataset}.mid"
        if not path.is_file():
            raise FileNotFoundError(f"benchmark dataset does not exist: {path}")
        data = path.read_bytes()
        # Neither contract calculation is passed to pyperf.  They are a
        # precondition only; timed functions below create fresh scores.
        symusic_contract = symusic_score_contract(parse_symusic_score(data))
        miso_contract = miso_score_contract(data)
        _same_contract(dataset, "miso", miso_contract, symusic_contract)
        if include_miso_unlimited:
            _same_contract(
                dataset,
                "miso-unlimited",
                _miso_unlimited_score_contract(data),
                symusic_contract,
            )
        prepared.append((dataset, data, symusic_contract))
    return prepared


def _forward_score_arguments(command: list[str], args) -> None:
    """Keep corpus selection identical in pyperf's worker subprocesses.

    pyperf intentionally reconstructs worker commands from its own flags, so
    application-specific argparse options are not forwarded automatically.
    """
    command.extend(("--corpus", str(args.corpus), "--datasets", *args.datasets))
    if args.include_miso_unlimited:
        command.append("--include-miso-unlimited")


def main() -> None:
    runner = pyperf.Runner(add_cmdline_args=_forward_score_arguments)
    runner.argparser.add_argument(
        "--corpus",
        type=Path,
        default=Path(__file__).with_name("corpus"),
        help="directory containing tiny.mid, normal.mid, huge.mid, and mahler.mid",
    )
    runner.argparser.add_argument(
        "--include-miso-unlimited",
        action="store_true",
        help=(
            "also time parse_score_unlimited as a trusted-input diagnostic; "
            "this does not change the checked Miso/Symusic headline"
        ),
    )
    runner.argparser.add_argument(
        "--datasets",
        nargs="+",
        choices=DEFAULT_DATASETS,
        default=list(DEFAULT_DATASETS),
        help="one or more warm in-memory datasets to benchmark",
    )
    args = runner.parse_args()

    try:
        prepared = _load_and_verify(args.corpus, args.datasets, args.include_miso_unlimited)
    except MisoScoreApiUnavailable as error:
        # Do not run Symusic alone: that would be a misleading competitive
        # result.  argparse.error provides a non-zero, actionable failure.
        runner.argparser.error(str(error))
    except Exception as error:
        # A mismatch or missing corpus is fatal before the first benchmark is
        # emitted, including in pyperf worker processes.
        runner.argparser.error(str(error))

    for dataset, data, contract in prepared:
        _set_metadata(runner, dataset, data, contract)
        # These functions receive bytes already resident in memory.  The return
        # value is not retained in a timed setup and digest generation is never
        # part of the measured body.
        runner.bench_func(f"miso/parse-score/{dataset}", _parse_miso_score, data)
        if args.include_miso_unlimited:
            runner.bench_func(
                f"miso-unlimited/parse-score/{dataset}", _parse_miso_unlimited_score, data
            )
        runner.bench_func(f"symusic/parse-score/{dataset}", parse_symusic_score, data)


def _parse_miso_score(data: bytes):
    """Resolve the score parser lazily so importing this harness is harmless."""
    import miso_midi

    return miso_midi.parse_score(data)


def _parse_miso_unlimited_score(data: bytes):
    """The timed trusted-input diagnostic operation; bytes are already warm."""
    import miso_midi

    return miso_midi.parse_score_unlimited(data)


if __name__ == "__main__":
    main()
