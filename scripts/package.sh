#!/usr/bin/env bash
set -euo pipefail
if (( $# != 4 )); then
    echo 'Usage: package.sh CONTROLLER OUTPUT_DIRECTORY X86_64_WORKER AARCH64_WORKER' >&2
    exit 2
fi
repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
controller=$(realpath -- "$1")
output=$(realpath -m -- "$2")
x86_worker=$(realpath -- "$3")
arm_worker=$(realpath -- "$4")
if [[ -e $output || -e $output.tar.gz ]]; then
    echo 'Package output already exists; choose an empty destination' >&2
    exit 1
fi
mkdir -p -- "$output"
install -m 0755 -- "$controller" "$output/rustsible"
cd -- "$repo_dir"
cargo run --locked -p rustsible-protocol --example worker_manifest -- "$output/workers" "x86_64=$x86_worker" "aarch64=$arm_worker"
install -m 0644 -- "$repo_dir/README.md" "$repo_dir/LICENSE" "$output/"
cd -- "$output"
sha256sum rustsible README.md LICENSE workers/manifest.json workers/rustsible-worker-linux-* > SHA256SUMS
sha256sum -c SHA256SUMS
cd -- "$(dirname -- "$output")"
tar -czf "$output.tar.gz" -- "$(basename -- "$output")"
