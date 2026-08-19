# Retained score-memory benchmark

`benchmarks/measure_score_memory.py` measures the resident memory required to
retain equivalent Miso and Symusic tick scores. It does not time parsing.

Run it on Linux only:

```bash
uv run --project benchmarks python benchmarks/measure_score_memory.py \
  --datasets tiny normal huge mahler \
  --count 64 \
  --output benchmarks/results/retained-score-memory.json
```

The driver first runs the complete `miso-score-contract/v1` preflight for every
requested corpus file. A semantic mismatch stops before any memory worker is
started. It then launches one fresh Python process for each
`(implementation, dataset)` pair. The worker imports only the selected
library, holds the input bytes, collects a post-import/input baseline, and
retains parsed score objects in a preallocated Python list.

The report contains:

- raw Linux current-RSS checkpoints from `/proc/self/statm`;
- an ordinary least-squares RSS slope over retained-score counts;
- final inclusive bytes per score, measured from the post-import/input
  baseline;
- Python list allocation, per-slot handle overhead, and the first score proxy
  object's `sys.getsizeof` value.

The slope isolates incremental score retention. The inclusive final number
explicitly includes the preallocated Python score-handle list, while the raw
checkpoints show page-granularity and allocator effects. Neither number is a
portable heap-accounting substitute: it is a Linux current-RSS measurement and
the tool deliberately refuses to run where that exact source is unavailable.

Use a count large enough to cross RSS page and allocator granularity. Checkpoint
counts default to powers of two plus the requested final count; override them
only with an increasing list ending in `--count`, for example
`--count 96 --checkpoints 1,2,4,8,16,32,64,96`.
