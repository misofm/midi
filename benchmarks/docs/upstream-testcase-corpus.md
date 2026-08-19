# Symusic v0.6.0 testcase differential corpus

This is a correctness-only, opt-in real-world differential set. Its manifest
pins exactly 27 MIDI files from Symusic tag `v0.6.0`, commit
`43ff25277abbc72dbd8d00fb5a9a14ec37fb7906`: all files in upstream
`tests/testcases/One_track_MIDIs` and `Multitrack_MIDIs` at that commit.

The repository stores the manifest, raw GitHub URLs, SHA-256 values, and full
`miso-score-contract/v1` digest/count expectations. It does **not** store MIDI
bytes. Fetch and verify them into the ignored local corpus directory with:

```bash
uv run --project benchmarks python -m benchmarks.symusic_testcases fetch
uv run --project benchmarks python -m benchmarks.symusic_testcases verify
uv run --project benchmarks python -m benchmarks.symusic_testcases differential \
  --evidence benchmarks/results/symusic-testcases-v0.6.0-equality.json
```

The last command constructs both Miso and installed Symusic 0.6 scores and
fails closed on any input checksum, canonical digest, or count mismatch. It is
not a benchmark and produces no timing result. To enable the matching pytest
test explicitly:

```bash
MISO_SYMUSIC_TESTCASES_CORPUS=benchmarks/corpus/symusic-v0.6.0-testcases \
  uv run --project benchmarks pytest \
    benchmarks/tests/test_symusic_testcases_corpus.py -q
```

Symusic is MIT-licensed, but that repository license does not itself establish
copyright or redistribution rights for the musical compositions or MIDI
arrangements in its test fixtures. For that reason this project keeps only the
metadata and downloads source-pinned bytes locally; assess any further use of
those bytes independently.
