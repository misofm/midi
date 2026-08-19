"""Python SDK for the Miso MIDI core."""

from os import PathLike
from pathlib import Path

from ._native import (
    MidiFile,
    ScanSummary,
    Score,
    ScoreParseLimits,
    parse,
    parse_score,
    parse_score_unlimited,
    scan,
)


def load(path: str | PathLike[str]) -> MidiFile:
    """Parse a Standard MIDI File from a filesystem path."""
    return parse(Path(path).read_bytes())


__all__ = [
    "MidiFile",
    "ScanSummary",
    "Score",
    "ScoreParseLimits",
    "load",
    "parse",
    "parse_score",
    "parse_score_unlimited",
    "scan",
]
