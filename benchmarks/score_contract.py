"""Canonical, equal-semantics contract for tick-based MIDI scores.

The parser benchmark deliberately compares *scores*, not parser-specific event
containers.  This module defines the exact observable score data used by that
comparison.  The wire encoding is canonical and independent of Python's
``repr`` and hash randomisation, so its SHA-256 digest is portable between
processes and bindings.

``semantic_records_from_symusic`` is intentionally allowed to inspect Symusic
objects.  ``miso_score_contract`` is not: it defines the bulk-only public
protocol required of the future Miso score API.
"""

from __future__ import annotations

from dataclasses import dataclass
from hashlib import sha256
import operator
from typing import Any, Mapping, Sequence


SCORE_CONTRACT_SCHEMA = "miso-score-contract/v1"
_MAGIC = b"MISO-SCORE-CONTRACT\x00\x01"
_SUMMARY_FIELDS = (
    "tracks",
    "notes",
    "controls",
    "pitch_bends",
    "pedals",
    "lyrics",
    "time_signatures",
    "key_signatures",
    "tempos",
    "markers",
)
_GLOBAL_FIELDS = ("time_signatures", "key_signatures", "tempos", "markers")
_TRACK_FIELDS = (
    "name",
    "program",
    "is_drum",
    "notes",
    "controls",
    "pitch_bends",
    "pedals",
    "lyrics",
)
_SCORE_FIELDS = ("tpq", "tracks", *_GLOBAL_FIELDS)


@dataclass(frozen=True)
class ScoreSummary:
    """Counts retained with every digest to diagnose semantic mismatches."""

    tracks: int
    notes: int
    controls: int
    pitch_bends: int
    pedals: int
    lyrics: int
    time_signatures: int
    key_signatures: int
    tempos: int
    markers: int

    def as_dict(self) -> dict[str, int]:
        return {field: getattr(self, field) for field in _SUMMARY_FIELDS}

    @classmethod
    def from_mapping(cls, value: Mapping[str, Any]) -> "ScoreSummary":
        _require_exact_keys(value, _SUMMARY_FIELDS, "summary")
        return cls(**{field: _integer(value[field], f"summary.{field}") for field in _SUMMARY_FIELDS})


@dataclass(frozen=True)
class ScoreContract:
    """Canonical bytes, digest, and output cardinalities for one score."""

    canonical: bytes
    sha256: str
    summary: ScoreSummary

    def metadata(self) -> dict[str, str | int]:
        """Return serialisable metadata suitable for a pyperf benchmark."""
        return {
            "contract_schema": SCORE_CONTRACT_SCHEMA,
            "contract_sha256": self.sha256,
            **{f"contract_{name}": value for name, value in self.summary.as_dict().items()},
        }


class MisoScoreApiUnavailable(RuntimeError):
    """Raised before timing when the public score API has not landed yet."""


def _require_exact_keys(value: Mapping[str, Any], expected: Sequence[str], label: str) -> None:
    actual = set(value)
    wanted = set(expected)
    if actual != wanted:
        missing = sorted(wanted - actual)
        extra = sorted(actual - wanted)
        raise ValueError(f"{label} keys differ; missing={missing}, extra={extra}")


def _integer(value: Any, label: str) -> int:
    """Normalise Python and NumPy integer scalars without accepting floats."""
    if isinstance(value, bool):
        raise TypeError(f"{label} must be an integer, not bool")
    try:
        result = operator.index(value)
    except TypeError as error:
        raise TypeError(f"{label} must be an integer, got {type(value).__name__}") from error
    if not -(1 << 63) <= result < (1 << 63):
        raise ValueError(f"{label} does not fit in signed 64 bits: {result}")
    return int(result)


def _text(value: Any, label: str) -> str:
    # NumPy string scalars intentionally become ordinary Python strings here.
    if not isinstance(value, str):
        raise TypeError(f"{label} must be text, got {type(value).__name__}")
    return str(value)


def _sequence(value: Any, label: str) -> Sequence[Any]:
    if isinstance(value, (str, bytes, bytearray)) or not isinstance(value, Sequence):
        raise TypeError(f"{label} must be an ordered sequence")
    return value


class _Encoder:
    def __init__(self) -> None:
        self.output = bytearray(_MAGIC)

    def integer(self, value: Any, label: str) -> None:
        self.output.extend(_integer(value, label).to_bytes(8, "big", signed=True))

    def count(self, value: int, label: str) -> None:
        if value < 0:
            raise ValueError(f"{label} cannot be negative")
        self.output.extend(value.to_bytes(8, "big", signed=False))

    def boolean(self, value: Any, label: str) -> None:
        if not isinstance(value, bool):
            raise TypeError(f"{label} must be bool")
        self.output.append(1 if value else 0)

    def text(self, value: Any, label: str) -> None:
        encoded = _text(value, label).encode("utf-8")
        self.count(len(encoded), f"{label} length")
        self.output.extend(encoded)


def semantic_records_from_symusic(score: Any) -> dict[str, Any]:
    """Extract the public Symusic tick-score semantics into Python builtins.

    The contract deliberately does not include unsupported SMF event kinds
    (for example SysEx), because Symusic's high-level ``Score`` does not retain
    them.  It does retain the listed score-level and per-track event families.
    """
    if type(getattr(score, "ttype", None)).__name__ != "Tick":
        raise TypeError("score contract accepts Symusic Tick scores only")

    def note(item: Any) -> dict[str, int]:
        return {
            "time": _integer(item.time, "note.time"),
            "duration": _integer(item.duration, "note.duration"),
            "pitch": _integer(item.pitch, "note.pitch"),
            "velocity": _integer(item.velocity, "note.velocity"),
        }

    def control(item: Any) -> dict[str, int]:
        return {
            "time": _integer(item.time, "control.time"),
            "number": _integer(item.number, "control.number"),
            "value": _integer(item.value, "control.value"),
        }

    def pitch_bend(item: Any) -> dict[str, int]:
        return {
            "time": _integer(item.time, "pitch_bend.time"),
            "value": _integer(item.value, "pitch_bend.value"),
        }

    def pedal(item: Any) -> dict[str, int]:
        return {
            "time": _integer(item.time, "pedal.time"),
            "duration": _integer(item.duration, "pedal.duration"),
        }

    def text(item: Any, label: str) -> dict[str, Any]:
        return {"time": _integer(item.time, f"{label}.time"), "text": _text(item.text, f"{label}.text")}

    tracks = []
    for track in score.tracks:
        tracks.append(
            {
                "name": _text(track.name, "track.name"),
                "program": _integer(track.program, "track.program"),
                "is_drum": bool(track.is_drum),
                "notes": [note(item) for item in track.notes],
                "controls": [control(item) for item in track.controls],
                "pitch_bends": [pitch_bend(item) for item in track.pitch_bends],
                "pedals": [pedal(item) for item in track.pedals],
                "lyrics": [text(item, "lyric") for item in track.lyrics],
            }
        )

    return {
        "tpq": _integer(score.ticks_per_quarter, "score.ticks_per_quarter"),
        "tracks": tracks,
        "time_signatures": [
            {
                "time": _integer(item.time, "time_signature.time"),
                "numerator": _integer(item.numerator, "time_signature.numerator"),
                "denominator": _integer(item.denominator, "time_signature.denominator"),
            }
            for item in score.time_signatures
        ],
        "key_signatures": [
            {
                "time": _integer(item.time, "key_signature.time"),
                "key": _integer(item.key, "key_signature.key"),
                "tonality": _integer(item.tonality, "key_signature.tonality"),
            }
            for item in score.key_signatures
        ],
        "tempos": [
            {"time": _integer(item.time, "tempo.time"), "mspq": _integer(item.mspq, "tempo.mspq")}
            for item in score.tempos
        ],
        "markers": [text(item, "marker") for item in score.markers],
    }


def _record(value: Any, expected: Sequence[str], label: str) -> Mapping[str, Any]:
    if not isinstance(value, Mapping):
        raise TypeError(f"{label} must be a mapping returned by semantic_records()")
    _require_exact_keys(value, expected, label)
    return value


def score_contract_from_records(records: Mapping[str, Any]) -> ScoreContract:
    """Encode the documented all-builtin ``Score.semantic_records()`` schema."""
    score = _record(records, _SCORE_FIELDS, "score")
    encoder = _Encoder()
    encoder.integer(score["tpq"], "score.tpq")

    counts = {field: 0 for field in _SUMMARY_FIELDS}
    tracks = _sequence(score["tracks"], "score.tracks")
    encoder.count(len(tracks), "score.tracks")
    counts["tracks"] = len(tracks)
    for track_index, raw_track in enumerate(tracks):
        track = _record(raw_track, _TRACK_FIELDS, f"track[{track_index}]")
        encoder.text(track["name"], f"track[{track_index}].name")
        encoder.integer(track["program"], f"track[{track_index}].program")
        encoder.boolean(track["is_drum"], f"track[{track_index}].is_drum")

        for field, event_fields in (
            ("notes", ("time", "duration", "pitch", "velocity")),
            ("controls", ("time", "number", "value")),
            ("pitch_bends", ("time", "value")),
            ("pedals", ("time", "duration")),
        ):
            events = _sequence(track[field], f"track[{track_index}].{field}")
            encoder.count(len(events), f"track[{track_index}].{field}")
            counts[field] += len(events)
            for event_index, raw_event in enumerate(events):
                event = _record(raw_event, event_fields, f"track[{track_index}].{field}[{event_index}]")
                for event_field in event_fields:
                    encoder.integer(event[event_field], f"track[{track_index}].{field}[{event_index}].{event_field}")

        lyrics = _sequence(track["lyrics"], f"track[{track_index}].lyrics")
        encoder.count(len(lyrics), f"track[{track_index}].lyrics")
        counts["lyrics"] += len(lyrics)
        for event_index, raw_event in enumerate(lyrics):
            event = _record(raw_event, ("time", "text"), f"track[{track_index}].lyrics[{event_index}]")
            encoder.integer(event["time"], f"track[{track_index}].lyrics[{event_index}].time")
            encoder.text(event["text"], f"track[{track_index}].lyrics[{event_index}].text")

    for field, event_fields in (
        ("time_signatures", ("time", "numerator", "denominator")),
        ("key_signatures", ("time", "key", "tonality")),
        ("tempos", ("time", "mspq")),
    ):
        events = _sequence(score[field], f"score.{field}")
        encoder.count(len(events), f"score.{field}")
        counts[field] = len(events)
        for event_index, raw_event in enumerate(events):
            event = _record(raw_event, event_fields, f"score.{field}[{event_index}]")
            for event_field in event_fields:
                encoder.integer(event[event_field], f"score.{field}[{event_index}].{event_field}")

    markers = _sequence(score["markers"], "score.markers")
    encoder.count(len(markers), "score.markers")
    counts["markers"] = len(markers)
    for event_index, raw_event in enumerate(markers):
        event = _record(raw_event, ("time", "text"), f"score.markers[{event_index}]")
        encoder.integer(event["time"], f"score.markers[{event_index}].time")
        encoder.text(event["text"], f"score.markers[{event_index}].text")

    canonical = bytes(encoder.output)
    return ScoreContract(
        canonical=canonical,
        sha256=sha256(canonical).hexdigest(),
        summary=ScoreSummary(**counts),
    )


def symusic_score_contract(score: Any) -> ScoreContract:
    """Return the v1 contract for a public Symusic ``ScoreTick`` object."""
    return score_contract_from_records(semantic_records_from_symusic(score))


def symusic_score_contract_from_midi(data: bytes) -> ScoreContract:
    """Load in-memory MIDI bytes into Symusic and calculate its contract."""
    from symusic import Score

    return symusic_score_contract(Score.from_midi(data))


def _contract_from_miso_digest(value: Any) -> ScoreContract:
    """Accept the future bulk ``Score.semantic_digest()`` response.

    Public Miso score bindings must return a mapping with exactly ``schema``,
    ``sha256`` and ``summary``.  Its digest is defined as this module's v1
    canonical bytes; the bytes need not cross FFI merely for benchmarking.
    """
    if not isinstance(value, Mapping):
        raise MisoScoreApiUnavailable(
            "miso_midi Score.semantic_digest() must return a mapping with "
            "schema, sha256, and summary"
        )
    _require_exact_keys(value, ("schema", "sha256", "summary"), "Score.semantic_digest()")
    if value["schema"] != SCORE_CONTRACT_SCHEMA:
        raise MisoScoreApiUnavailable(
            f"Miso score digest schema must be {SCORE_CONTRACT_SCHEMA!r}, got {value['schema']!r}"
        )
    digest = value["sha256"]
    if not isinstance(digest, str) or len(digest) != 64 or any(c not in "0123456789abcdef" for c in digest):
        raise MisoScoreApiUnavailable("Miso Score.semantic_digest() returned an invalid lowercase SHA-256")
    return ScoreContract(canonical=b"", sha256=digest, summary=ScoreSummary.from_mapping(value["summary"]))


def miso_score_contract(data: bytes) -> ScoreContract:
    """Get the future Miso score contract without per-event Python access.

    The currently shipped Miso package intentionally has no score parser.  The
    exception is explicit so the comparison harness refuses to emit a partial
    or non-equivalent benchmark.  Once ``parse_score`` exists, the binding may
    expose either the bulk digest protocol above or all-builtin records.
    """
    import miso_midi

    parse_score = getattr(miso_midi, "parse_score", None)
    if not callable(parse_score):
        raise MisoScoreApiUnavailable(
            "miso_midi.parse_score(bytes) is not available. Implement the public score API "
            "and Score.semantic_digest() (preferred) or Score.semantic_records() before "
            "running an equal-semantics score benchmark."
        )
    score = parse_score(data)
    semantic_digest = getattr(score, "semantic_digest", None)
    if callable(semantic_digest):
        return _contract_from_miso_digest(semantic_digest())
    semantic_records = getattr(score, "semantic_records", None)
    if callable(semantic_records):
        return score_contract_from_records(semantic_records())
    raise MisoScoreApiUnavailable(
        "miso_midi Score must expose bulk semantic_digest() or semantic_records(); "
        "the benchmark will not inspect individual events over Python FFI."
    )
