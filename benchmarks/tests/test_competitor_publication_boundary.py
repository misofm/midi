"""Keep competitor references confined to the benchmark project."""

from __future__ import annotations

from pathlib import Path
import subprocess


_COMPETITOR_NAMES = (
    "mido",
    "symusic",
    "miditoolkit",
    "pretty_midi",
    "music21",
    "midi.jl",
)


def _tracked_text_files(repository: Path) -> list[Path]:
    listing = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=repository,
        check=True,
        capture_output=True,
    ).stdout
    paths: list[Path] = []
    for encoded_path in listing.split(b"\0"):
        if not encoded_path:
            continue
        relative = Path(encoded_path.decode("utf-8"))
        if relative.parts and relative.parts[0] == "benchmarks":
            continue
        path = repository / relative
        if path.is_file() and b"\0" not in path.read_bytes():
            paths.append(path)
    return paths


def test_competitor_names_are_confined_to_benchmark_paths() -> None:
    repository = Path(__file__).resolve().parents[2]
    offenders: list[str] = []
    for path in _tracked_text_files(repository):
        content = path.read_text(encoding="utf-8", errors="ignore").casefold()
        found = [name for name in _COMPETITOR_NAMES if name in content]
        if found:
            offenders.append(f"{path.relative_to(repository)}: {', '.join(found)}")

    assert not offenders, "competitor names must remain under benchmarks/**:\n" + "\n".join(offenders)
