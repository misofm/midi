#!/usr/bin/env bash
# Run only on a benchmark host. This creates new raw outputs and intentionally
# does not overwrite a checked-in final artifact.
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "${script_dir}/../.." && pwd)
benchmark_project="${repo_root}/benchmarks"
output_dir=${MISO_NATIVE_OUTPUT_DIR:-"${repo_root}/benchmarks/results/native-score-local"}
affinity=${MISO_NATIVE_AFFINITY:-4}
datasets=${MISO_NATIVE_DATASETS:-tiny,normal,huge,mahler}
samples=${MISO_NATIVE_SAMPLES:-30}
warmup=${MISO_NATIVE_WARMUP:-5}
iterations=${MISO_NATIVE_ITERATIONS:-0}
min_sample_ns=${MISO_NATIVE_MIN_SAMPLE_NS:-50000000}

mkdir -p -- "${output_dir}"

# Keep the public Python contract checker in sync with this checkout. CMake,
# Ninja, and Maturin are pinned uv development tools, not production deps.
uv run --project "${benchmark_project}" maturin develop --release
uv run --project "${benchmark_project}" bash "${script_dir}/fetch_symusic.sh"

symusic_binary=${MISO_SYMUSIC_BINARY:-"${MISO_SYMUSIC_BUILD_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/miso-midi/symusic-build-43ff25277abbc72dbd8d00fb5a9a14ec37fb7906-ninja}/miso_native_symusic_bench"}

uv run --project "${benchmark_project}" python -m benchmarks.native_symusic.preflight \
  --datasets ${datasets//,/ } \
  --output "${output_dir}/preflight.json"

run_miso() {
  taskset -c "${affinity}" cargo run -p miso-midi-native-score-bench --release -- \
    --datasets "${datasets}" --samples "${samples}" --warmup "${warmup}" \
    --iterations "${iterations}" --min-sample-ns "${min_sample_ns}" --parse-only \
    --output "$1"
}
run_symusic() {
  taskset -c "${affinity}" "${symusic_binary}" \
    --datasets "${datasets}" --samples "${samples}" --warmup "${warmup}" \
    --iterations "${iterations}" --min-sample-ns "${min_sample_ns}" \
    --output "$1"
}

# ABBA keeps first/last-run drift from belonging to only one implementation.
run_miso "${output_dir}/miso-a.json"
run_symusic "${output_dir}/symusic-a.json"
run_symusic "${output_dir}/symusic-b.json"
run_miso "${output_dir}/miso-b.json"

uv run --project "${benchmark_project}" python -m benchmarks.native_symusic.combine \
  --preflight "${output_dir}/preflight.json" \
  --miso "${output_dir}/miso-a.json" --miso "${output_dir}/miso-b.json" \
  --symusic "${output_dir}/symusic-a.json" --symusic "${output_dir}/symusic-b.json" \
  --output "${output_dir}/comparison.json"

printf '%s\n' "native comparison written under ${output_dir}"
