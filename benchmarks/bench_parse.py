from __future__ import annotations

from io import BytesIO
from pathlib import Path

import mido
import pyperf

from miso_midi import parse, scan


def mido_parse_objects(data: bytes) -> mido.MidiFile:
    return mido.MidiFile(file=BytesIO(data))


def miso_semantic_records(data: bytes) -> list[tuple]:
    return parse(data).semantic_records()


def _skip_vlq(data: bytes, position: int) -> int:
    while data[position] & 0x80:
        position += 1
    return position + 1


def mido_semantic_records(data: bytes) -> list[tuple]:
    midi = mido_parse_objects(data)
    records = []
    for track_index, track in enumerate(midi.tracks):
        for message in track:
            if message.is_meta:
                encoded = bytes(message.bytes())
                payload_start = _skip_vlq(encoded, 2)
                records.append(
                    (track_index, message.time, 0xFF, encoded[1], encoded[payload_start:])
                )
            elif message.type == "sysex":
                records.append(
                    (track_index, message.time, 0xF0, None, bytes(message.data))
                )
            else:
                encoded = bytes(message.bytes())
                records.append(
                    (track_index, message.time, encoded[0], None, encoded[1:])
                )
    return records


def main() -> None:
    runner = pyperf.Runner()
    runner.argparser.add_argument(
        "--corpus",
        type=Path,
        default=Path(__file__).with_name("corpus"),
    )
    runner.argparser.add_argument(
        "--datasets",
        nargs="+",
        default=["tiny", "normal", "huge", "mahler"],
    )
    args = runner.parse_args()

    for name in args.datasets:
        path = args.corpus / f"{name}.mid"
        data = path.read_bytes()
        runner.metadata[f"{name}_bytes"] = len(data)
        runner.bench_func(f"miso/scan/{name}", scan, data)
        runner.bench_func(f"miso/parse-arena/{name}", parse, data)
        runner.bench_func(f"mido/parse-objects/{name}", mido_parse_objects, data)
        runner.bench_func(f"miso/semantic-records/{name}", miso_semantic_records, data)
        runner.bench_func(f"mido/semantic-records/{name}", mido_semantic_records, data)

if __name__ == "__main__":
    main()
