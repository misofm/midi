"""Full equal-semantics preflight for the native Rust/C++ comparison.

This deliberately runs before either timed native binary.  It proves that the
current public Miso binding and Symusic 0.6 score API agree with the fixed v1
contract for the selected warm-byte corpus, while the C++ harness independently
recomputes the Symusic contract from its source-pinned C++ score.
"""

from __future__ import annotations

import argparse
from hashlib import sha256
import importlib.metadata
import json
from pathlib import Path
from typing import Any

from benchmarks.score_contract import SCORE_CONTRACT_SCHEMA, ScoreSummary, miso_score_contract, symusic_score_contract_from_midi


EXPECTED: dict[str, tuple[str, str, ScoreSummary]] = {
    "tiny": (
        "39da22e3a55fdf78b68855e8ed870ccfbf3e5d077401fba7174773f7fa7c92d7",
        "bd36b66d133db7772eb2bc5e81e7a1c9ea4a62561de0131a9465ba73c9491acc",
        ScoreSummary(1, 16, 3, 1, 1, 0, 0, 0, 0, 1),
    ),
    "normal": (
        "4b62f8bbd60175f610097817e1759514297f694a46320e1f3d770dbb88c94f97",
        "d75cb3bb06a230b8bbbb371e32cf86f5aeaa2a4c1ea098f7f5f371eb559271f1",
        ScoreSummary(8, 16_000, 272, 64, 8, 0, 0, 0, 0, 32),
    ),
    "huge": (
        "90d7ad33e14e80149d8cd2c3d0dae204de9b2ec4670b850593864111245bd40f",
        "fe10b416f2f7a65925f38e2a66f201b427040c3243d2b7c818bde3297b12d37c",
        ScoreSummary(16, 192_000, 3_040, 752, 16, 0, 0, 0, 0, 384),
    ),
    "mahler": (
        "35a59329ab8f1f86ec2602bb5293b9fbddc694e512aafa00e310cb8da237f302",
        "d8fcfebd208541d7791fc0dab49b561893a7c50180ccbcc61b7049e009013f69",
        ScoreSummary(51, 60_411, 36_287, 0, 0, 0, 97, 97, 177, 97),
    ),
}


def preflight(corpus_dir: Path, datasets: list[str]) -> dict[str, Any]:
    """Return metadata only after both public score APIs match fixed semantics."""
    results: dict[str, Any] = {}
    for name in datasets:
        try:
            expected_input, expected_digest, expected_summary = EXPECTED[name]
        except KeyError as error:
            raise ValueError(f"unknown dataset {name!r}; expected {', '.join(EXPECTED)}") from error
        data = (corpus_dir / f"{name}.mid").read_bytes()
        actual_input = sha256(data).hexdigest()
        if actual_input != expected_input:
            raise ValueError(f"{name}: corpus SHA-256 differs from fixed expectation")
        miso = miso_score_contract(data)
        symusic = symusic_score_contract_from_midi(data)
        if miso.sha256 != symusic.sha256 or miso.summary != symusic.summary:
            raise ValueError(f"{name}: Miso and Symusic public contracts differ")
        if miso.sha256 != expected_digest or miso.summary != expected_summary:
            raise ValueError(f"{name}: public score contract differs from fixed v1 expectation")
        results[name] = {
            "input_bytes": len(data),
            "input_sha256": actual_input,
            "semantic_contract": {
                "schema": SCORE_CONTRACT_SCHEMA,
                "sha256": expected_digest,
                "summary": expected_summary.as_dict(),
            },
        }
    return {
        "schema": "miso-native-score-preflight/v1",
        "miso_midi_version": importlib.metadata.version("miso-midi"),
        "symusic_version": importlib.metadata.version("symusic"),
        "datasets": results,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus-dir", type=Path, default=Path("benchmarks/corpus"))
    parser.add_argument("--datasets", nargs="+", default=list(EXPECTED))
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = preflight(args.corpus_dir, args.datasets)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
