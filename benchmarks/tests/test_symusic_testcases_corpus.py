"""Offline validation for the opt-in checksum-pinned Symusic testcase corpus."""

from __future__ import annotations

from copy import deepcopy
from hashlib import sha256
import os
from pathlib import Path

import pytest

from benchmarks.symusic_testcases import (
    CorpusVerificationError,
    differential,
    fetch_corpus,
    load_manifest,
    validate_manifest,
    verify_corpus,
)


def _offline_manifest() -> dict:
    manifest = deepcopy(load_manifest())
    payload = b"offline-pinned-bytes"
    digest = sha256(payload).hexdigest()
    for entry in manifest["files"]:
        entry["input_bytes"] = len(payload)
        entry["input_sha256"] = digest
        entry["contract"]["sha256"] = "0" * 64
        entry["contract"]["summary"] = {
            "tracks": 0,
            "notes": 0,
            "controls": 0,
            "pitch_bends": 0,
            "pedals": 0,
            "lyrics": 0,
            "time_signatures": 0,
            "key_signatures": 0,
            "tempos": 0,
            "markers": 0,
        }
    return manifest


def test_manifest_is_complete_commit_pinned_and_has_all_selected_files() -> None:
    manifest = load_manifest()
    validate_manifest(manifest)

    assert manifest["source"]["commit"] == "43ff25277abbc72dbd8d00fb5a9a14ec37fb7906"
    assert len(manifest["files"]) == 27
    assert sum("/One_track_MIDIs/" in entry["path"] for entry in manifest["files"]) == 17
    assert sum("/Multitrack_MIDIs/" in entry["path"] for entry in manifest["files"]) == 10
    assert all("raw.githubusercontent.com" in entry["raw_url"] for entry in manifest["files"])


def test_fetch_and_verify_checksum_validation_need_no_network(tmp_path: Path) -> None:
    manifest = _offline_manifest()
    payload = b"offline-pinned-bytes"
    calls: list[str] = []

    paths = fetch_corpus(manifest, tmp_path, download=lambda url: calls.append(url) or payload)
    assert len(paths) == 27
    assert len(calls) == 27
    assert verify_corpus(manifest, tmp_path) == paths

    paths[0].write_bytes(b"altered")
    with pytest.raises(CorpusVerificationError, match="does not match manifest"):
        verify_corpus(manifest, tmp_path)


def test_fetch_fails_closed_before_writing_unchecked_bytes(tmp_path: Path) -> None:
    manifest = _offline_manifest()
    with pytest.raises(CorpusVerificationError, match="SHA-256 differs"):
        fetch_corpus(manifest, tmp_path, download=lambda _: b"x" * len(b"offline-pinned-bytes"))
    assert not list(tmp_path.rglob("*.mid"))


@pytest.mark.skipif(
    not os.environ.get("MISO_SYMUSIC_TESTCASES_CORPUS"),
    reason="set MISO_SYMUSIC_TESTCASES_CORPUS to opt into the downloaded real-world corpus",
)
def test_opt_in_real_world_symusic_differential() -> None:
    report = differential(load_manifest(), Path(os.environ["MISO_SYMUSIC_TESTCASES_CORPUS"]))
    assert report["files_verified"] == 27
    assert report["result"] == "full_miso_symusic_contract_equality"
