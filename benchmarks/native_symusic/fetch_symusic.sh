#!/usr/bin/env bash
# Fetch exactly the source used by the native Symusic comparison. This script
# never downloads a wheel or stores a prebuilt competitor binary in this repo.
set -euo pipefail

readonly symusic_url='https://github.com/Yikai-Liao/symusic.git'
readonly symusic_commit='43ff25277abbc72dbd8d00fb5a9a14ec37fb7906' # tag: v0.6.0
readonly submodules=(
  '3rdparty/Catch2:29c9844f688acb27c87338c39cd186ebfe41aa19'
  '3rdparty/abcmidi:7ba0b738e9bcd504288758472380e1597208a7a8'
  '3rdparty/fmt:407c905e45ad75fc29bf0f9bb7c5c2fd3475976f'
  '3rdparty/minimidi:3d62d6e3851a5e761a9a562d3cd306a70acfeebc'
  '3rdparty/nanobench:a5a50c2b33eea2ff1fcb355cacdface43eb42b25'
  '3rdparty/nanobind:2a61ad2494d09fecb2e13322c1383342c299900d'
  '3rdparty/pdqsort:b1ef26a55cdb60d236a5cb199c4234c704f46726'
  '3rdparty/prestosynth:424421b997fcef0db71b64cabaecd5b70b86b831'
  '3rdparty/pyvec:347e8796950b8c5f57d63dc137a157b4ecdba896'
  '3rdparty/zpp_bits:efdfd613556efccfa6377d2c880ff0b3048182dd'
)

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "${script_dir}/../.." && pwd)
cache_root=${MISO_SYMUSIC_CACHE_DIR:-"${XDG_CACHE_HOME:-$HOME/.cache}/miso-midi"}
source_dir=${MISO_SYMUSIC_SOURCE_DIR:-"${cache_root}/symusic-${symusic_commit}"}
# The Ninja suffix deliberately makes a pre-existing non-Ninja CMake cache a
# different build directory.  This avoids silently reusing an environment-
# dependent generator after the benchmark toolchain was pinned in uv.
build_dir=${MISO_SYMUSIC_BUILD_DIR:-"${cache_root}/symusic-build-${symusic_commit}-ninja"}

if [[ ! -d "${source_dir}/.git" ]]; then
  mkdir -p -- "$(dirname -- "${source_dir}")"
  git clone --filter=blob:none "${symusic_url}" "${source_dir}"
elif ! git -C "${source_dir}" diff --quiet || ! git -C "${source_dir}" diff --cached --quiet; then
  printf '%s\n' "Refusing to reuse dirty source cache: ${source_dir}" >&2
  exit 1
fi

if [[ -n "$(git -C "${source_dir}" status --porcelain --untracked-files=all)" ]]; then
  printf '%s\n' "Refusing to reuse dirty source cache: ${source_dir}" >&2
  exit 1
fi

reject_dirty_submodules() {
  local dirty_path
  while IFS= read -r dirty_path; do
    [[ -z "${dirty_path}" ]] || {
      printf 'Refusing to build with dirty Symusic submodule: %s\n' "${dirty_path}" >&2
      exit 1
    }
  done < <(git -C "${source_dir}" submodule foreach --recursive --quiet \
    'if test -n "$(git status --porcelain --untracked-files=all)"; then printf "%s\n" "$sm_path"; fi')
}

# Check existing initialized modules before update as well as the final tree:
# an update must never be an opportunity to reuse or mask a dirty cache.
reject_dirty_submodules

git -C "${source_dir}" fetch --quiet origin "${symusic_commit}"
git -C "${source_dir}" checkout --detach --quiet "${symusic_commit}"
git -C "${source_dir}" submodule sync --recursive
git -C "${source_dir}" submodule update --init --recursive

actual_commit=$(git -C "${source_dir}" rev-parse HEAD)
[[ "${actual_commit}" == "${symusic_commit}" ]] || {
  printf 'Symusic commit mismatch: expected %s, got %s\n' "${symusic_commit}" "${actual_commit}" >&2
  exit 1
}

for entry in "${submodules[@]}"; do
  path=${entry%%:*}
  expected=${entry#*:}
  actual=$(git -C "${source_dir}/${path}" rev-parse HEAD)
  [[ "${actual}" == "${expected}" ]] || {
    printf 'Submodule mismatch for %s: expected %s, got %s\n' "${path}" "${expected}" "${actual}" >&2
    exit 1
  }
done

reject_dirty_submodules

uv run --project "${repo_root}/benchmarks" cmake -G Ninja -S "${script_dir}" -B "${build_dir}" \
  -DSYMUSIC_SOURCE_DIR="${source_dir}" \
  -DMISO_SYMUSIC_PIN="${symusic_commit}" \
  -DCMAKE_BUILD_TYPE=Release
uv run --project "${repo_root}/benchmarks" cmake --build "${build_dir}" --target miso_native_symusic_bench --parallel

printf '%s\n' "source=${source_dir}"
printf '%s\n' "build=${build_dir}"
printf '%s\n' "binary=${build_dir}/miso_native_symusic_bench"
