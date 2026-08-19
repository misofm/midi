"""Checksum-pinned, opt-in differential corpus from Symusic v0.6.0 tests.

This module deliberately downloads no bytes at import time.  The manifest
contains hashes and score-contract expectations, while the MIDI files remain
in an ignored local directory.
"""

from __future__ import annotations

import argparse
from collections.abc import Callable, Iterable, Mapping
from hashlib import sha256
import json
from pathlib import Path
from tempfile import NamedTemporaryFile
from typing import Any
from urllib.parse import quote
from urllib.request import urlopen

from benchmarks.score_contract import SCORE_CONTRACT_SCHEMA, score_contract_from_records, symusic_score_contract


SCHEMA = "miso-symusic-testcases-corpus/v1"
REPOSITORY = "Yikai-Liao/symusic"
COMMIT = "43ff25277abbc72dbd8d00fb5a9a14ec37fb7906"
RAW_BASE_URL = f"https://raw.githubusercontent.com/{REPOSITORY}/{COMMIT}/"
MANIFEST_PATH = Path(__file__).with_name("symusic_testcases_manifest.json")
DEFAULT_CORPUS = Path(__file__).with_name("corpus") / "symusic-v0.6.0-testcases"
SUMMARY_FIELDS = (
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
SELECTED_PATHS = (
    "tests/testcases/Multitrack_MIDIs/Aicha.mid",
    "tests/testcases/Multitrack_MIDIs/All The Small Things.mid",
    "tests/testcases/Multitrack_MIDIs/Funkytown.mid",
    "tests/testcases/Multitrack_MIDIs/Girls Just Want to Have Fun.mid",
    "tests/testcases/Multitrack_MIDIs/I Gotta Feeling.mid",
    "tests/testcases/Multitrack_MIDIs/In Too Deep.mid",
    "tests/testcases/Multitrack_MIDIs/Les Yeux Revolvers.mid",
    "tests/testcases/Multitrack_MIDIs/Mr. Blue Sky.mid",
    "tests/testcases/Multitrack_MIDIs/Shut Up.mid",
    "tests/testcases/Multitrack_MIDIs/What a Fool Believes.mid",
    "tests/testcases/One_track_MIDIs/6338816_Etude No. 4.mid",
    "tests/testcases/One_track_MIDIs/6354774_Macabre Waltz.mid",
    *(f"tests/testcases/One_track_MIDIs/Maestro_{index}.mid" for index in (1, 10, 2, 3, 4, 5, 6, 7, 8, 9)),
    "tests/testcases/One_track_MIDIs/POP909_008.mid",
    "tests/testcases/One_track_MIDIs/POP909_010.mid",
    "tests/testcases/One_track_MIDIs/POP909_022.mid",
    "tests/testcases/One_track_MIDIs/POP909_191.mid",
    "tests/testcases/One_track_MIDIs/empty.mid",
)


class CorpusVerificationError(RuntimeError):
    """A source, checksum, or semantic contract did not match the manifest."""


def raw_url(path: str) -> str:
    """Return the only allowed byte source for one relative upstream path."""
    if not path.startswith("tests/testcases/") or path.startswith("/") or ".." in path.split("/"):
        raise ValueError(f"unsafe upstream testcase path: {path!r}")
    return RAW_BASE_URL + quote(path, safe="/")


def _contract_record(contract: Any) -> dict[str, Any]:
    return {
        "schema": SCORE_CONTRACT_SCHEMA,
        "sha256": contract.sha256,
        "summary": contract.summary.as_dict(),
    }


def _validate_contract(value: Mapping[str, Any], label: str, *, complete: bool) -> None:
    expected_keys = {"schema", "sha256", "summary"}
    if set(value) != expected_keys:
        raise ValueError(f"{label}.contract keys must be {sorted(expected_keys)}")
    if value["schema"] != SCORE_CONTRACT_SCHEMA:
        raise ValueError(f"{label}.contract schema is not {SCORE_CONTRACT_SCHEMA!r}")
    digest = value["sha256"]
    if not isinstance(digest, str) or (complete and (len(digest) != 64 or set(digest) - set("0123456789abcdef"))):
        raise ValueError(f"{label}.contract sha256 is invalid")
    summary = value["summary"]
    if not isinstance(summary, Mapping) or set(summary) != set(SUMMARY_FIELDS):
        raise ValueError(f"{label}.contract summary fields are invalid")
    if any(not isinstance(summary[field], int) or summary[field] < 0 for field in SUMMARY_FIELDS):
        raise ValueError(f"{label}.contract summary values must be non-negative integers")


def validate_manifest(value: Mapping[str, Any], *, complete: bool = True) -> None:
    """Validate pinned source identity, paths, hashes, and contract schema."""
    expected_keys = {"schema", "source", "license_notice", "files"}
    if set(value) != expected_keys:
        raise ValueError(f"manifest keys must be {sorted(expected_keys)}")
    if value["schema"] != SCHEMA:
        raise ValueError(f"manifest schema is not {SCHEMA!r}")
    source = value["source"]
    if not isinstance(source, Mapping) or source != {
        "repository": REPOSITORY,
        "commit": COMMIT,
        "tag": "v0.6.0",
    }:
        raise ValueError("manifest source must pin the exact Symusic v0.6.0 commit")
    if not isinstance(value["license_notice"], str) or not value["license_notice"]:
        raise ValueError("manifest must include a non-empty source/license notice")
    files = value["files"]
    if not isinstance(files, list) or len(files) != 27:
        raise ValueError("manifest must contain exactly 27 selected upstream test files")

    paths: list[str] = []
    for index, entry in enumerate(files):
        label = f"files[{index}]"
        if not isinstance(entry, Mapping) or set(entry) != {
            "path", "raw_url", "input_bytes", "input_sha256", "contract"
        }:
            raise ValueError(f"{label} has invalid keys")
        path = entry["path"]
        if not isinstance(path, str):
            raise ValueError(f"{label}.path must be text")
        expected_url = raw_url(path)
        if entry["raw_url"] != expected_url:
            raise ValueError(f"{label}.raw_url is not the commit-pinned raw GitHub URL")
        if not isinstance(entry["input_bytes"], int) or entry["input_bytes"] < 0:
            raise ValueError(f"{label}.input_bytes must be non-negative")
        digest = entry["input_sha256"]
        if not isinstance(digest, str) or (complete and (len(digest) != 64 or set(digest) - set("0123456789abcdef"))):
            raise ValueError(f"{label}.input_sha256 is invalid")
        _validate_contract(entry["contract"], label, complete=complete)
        paths.append(path)
    if paths != sorted(paths):
        raise ValueError("manifest files must be sorted by upstream path")
    if len(paths) != len(set(paths)):
        raise ValueError("manifest contains duplicate upstream paths")
    expected_prefixes = {"tests/testcases/One_track_MIDIs/", "tests/testcases/Multitrack_MIDIs/"}
    if {next(prefix for prefix in expected_prefixes if path.startswith(prefix)) for path in paths} != expected_prefixes:
        raise ValueError("manifest must cover the selected one-track and multitrack directories")


def load_manifest(path: Path = MANIFEST_PATH) -> dict[str, Any]:
    value = json.loads(path.read_text())
    if not isinstance(value, dict):
        raise ValueError("manifest root must be an object")
    validate_manifest(value)
    return value


def corpus_path(output: Path, upstream_path: str) -> Path:
    return output / Path(upstream_path).relative_to("tests/testcases")


def _atomic_write(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with NamedTemporaryFile(dir=path.parent, delete=False) as temporary:
        temporary.write(data)
        temporary_path = Path(temporary.name)
    temporary_path.replace(path)


def fetch_corpus(
    manifest: Mapping[str, Any],
    output: Path,
    *,
    download: Callable[[str], bytes] | None = None,
) -> list[Path]:
    """Fetch only pinned bytes and verify each before placing it locally."""
    validate_manifest(manifest)
    if download is None:
        def download(url: str) -> bytes:
            with urlopen(url, timeout=30) as response:
                return response.read()

    paths = []
    for entry in manifest["files"]:
        data = download(entry["raw_url"])
        if len(data) != entry["input_bytes"]:
            raise CorpusVerificationError(f"{entry['path']}: byte count differs from manifest")
        digest = sha256(data).hexdigest()
        if digest != entry["input_sha256"]:
            raise CorpusVerificationError(f"{entry['path']}: SHA-256 differs from manifest")
        path = corpus_path(output, entry["path"])
        _atomic_write(path, data)
        paths.append(path)
    return paths


def verify_corpus(manifest: Mapping[str, Any], output: Path) -> list[Path]:
    """Fail closed if the ignored local corpus is absent or altered."""
    validate_manifest(manifest)
    paths = []
    for entry in manifest["files"]:
        path = corpus_path(output, entry["path"])
        if not path.is_file():
            raise CorpusVerificationError(f"missing corpus file: {path}")
        data = path.read_bytes()
        if len(data) != entry["input_bytes"] or sha256(data).hexdigest() != entry["input_sha256"]:
            raise CorpusVerificationError(f"{entry['path']}: local corpus does not match manifest")
        paths.append(path)
    return paths


def differential(manifest: Mapping[str, Any], output: Path) -> dict[str, Any]:
    """Compare the full Miso/Symusic score contract without any timing."""
    from miso_midi import parse_score
    from symusic import Score

    verify_corpus(manifest, output)
    totals = {field: 0 for field in SUMMARY_FIELDS}
    input_bytes = 0
    for entry in manifest["files"]:
        data = corpus_path(output, entry["path"]).read_bytes()
        miso = score_contract_from_records(parse_score(data).semantic_records())
        symusic = symusic_score_contract(Score.from_midi(data))
        expected = entry["contract"]
        actual = _contract_record(miso)
        if actual != expected:
            raise CorpusVerificationError(f"{entry['path']}: Miso contract differs from manifest")
        if _contract_record(symusic) != expected or symusic != miso:
            raise CorpusVerificationError(f"{entry['path']}: Symusic contract differs from Miso/manifest")
        input_bytes += entry["input_bytes"]
        for field in SUMMARY_FIELDS:
            totals[field] += expected["summary"][field]
    return {
        "schema": "miso-symusic-testcases-evidence/v1",
        "source": manifest["source"],
        "score_contract_schema": SCORE_CONTRACT_SCHEMA,
        "manifest_sha256": sha256(
            json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()
        ).hexdigest(),
        "files_verified": len(manifest["files"]),
        "input_bytes_verified": input_bytes,
        "summary_totals": totals,
        "result": "full_miso_symusic_contract_equality",
    }


def bootstrap_manifest(path: Path) -> None:
    """Maintainer-only refresh from the pinned commit; does not vendor MIDI."""
    from miso_midi import parse_score
    from symusic import Score

    files = []
    for upstream_path in SELECTED_PATHS:
        url = raw_url(upstream_path)
        with urlopen(url, timeout=30) as response:
            data = response.read()
        miso = score_contract_from_records(parse_score(data).semantic_records())
        symusic = symusic_score_contract(Score.from_midi(data))
        if miso != symusic:
            raise CorpusVerificationError(f"{upstream_path}: Miso and Symusic differ during manifest refresh")
        files.append(
            {
                "path": upstream_path,
                "raw_url": url,
                "input_bytes": len(data),
                "input_sha256": sha256(data).hexdigest(),
                "contract": _contract_record(miso),
            }
        )
    manifest = {
        "schema": SCHEMA,
        "source": {"repository": REPOSITORY, "commit": COMMIT, "tag": "v0.6.0"},
        "license_notice": (
            "Source files are fetched from Symusic v0.6.0 testcases. Symusic is MIT-licensed; "
            "this manifest does not assert copyright or redistribution rights for the musical compositions "
            "or MIDI arrangements, so MIDI bytes are not vendored."
        ),
        "files": files,
    }
    validate_manifest(manifest)
    _write_json(path, manifest)


def _write_json(path: Path, value: Mapping[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("bootstrap-manifest", "fetch", "verify", "differential"))
    parser.add_argument("--manifest", type=Path, default=MANIFEST_PATH)
    parser.add_argument("--output", type=Path, default=DEFAULT_CORPUS)
    parser.add_argument("--evidence", type=Path, help="write the correctness-only differential evidence JSON")
    args = parser.parse_args()
    if args.command == "bootstrap-manifest":
        bootstrap_manifest(args.manifest)
        print(f"wrote checksum-pinned manifest: {args.manifest}")
        return
    manifest = load_manifest(args.manifest)
    if args.command == "fetch":
        paths = fetch_corpus(manifest, args.output)
        print(f"fetched and verified {len(paths)} files into {args.output}")
    elif args.command == "verify":
        paths = verify_corpus(manifest, args.output)
        print(f"verified {len(paths)} files in {args.output}")
    else:
        report = differential(manifest, args.output)
        if args.evidence:
            _write_json(args.evidence, report)
        print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
