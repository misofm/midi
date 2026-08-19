from __future__ import annotations

from io import BytesIO
from pathlib import Path

import mido
import pyperf

from miso_midi import scan


def miso_reject(data: bytes) -> None:
    try:
        scan(data)
    except ValueError:
        return
    raise AssertionError("malformed input was accepted")


def mido_attempt(data: bytes) -> None:
    try:
        mido.MidiFile(file=BytesIO(data))
    except (EOFError, OSError, ValueError):
        pass


def main() -> None:
    runner = pyperf.Runner()
    runner.argparser.add_argument(
        "--corpus",
        type=Path,
        default=Path(__file__).with_name("corpus"),
    )
    args = runner.parse_args()

    for path in sorted(args.corpus.glob("malformed-*.mid")):
        data = path.read_bytes()
        name = path.stem.removeprefix("malformed-")
        runner.bench_func(f"miso/reject/{name}", miso_reject, data)
        # Some malformed cases are intentionally accepted by Mido, so this is
        # named as an attempt rather than a rejection benchmark.
        runner.bench_func(f"mido/attempt/{name}", mido_attempt, data)


if __name__ == "__main__":
    main()
