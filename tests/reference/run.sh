#!/usr/bin/env bash
set -euo pipefail
repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
binary=$(realpath -- "${1:-$repo_dir/target/debug/rustsible}")
fixture_dir=$(mktemp -d /tmp/rustsible-reference.XXXXXXXX)
cleanup() {
    local result=$?
    if (( result != 0 )); then
        for log in "$fixture_dir"/*.log; do [[ ! -f $log ]] || cat "$log" >&2; done
    fi
    rm -rf -- "$fixture_dir"
}
trap cleanup EXIT
printf '[all]\nlocalhost ansible_connection=local\n' > "$fixture_dir/inventory"
cp "$repo_dir/tests/reference/input.j2" "$fixture_dir/input.j2"
for name in template non_boolean; do
    sed "s|@OUTPUT@|$fixture_dir/result|g" "$repo_dir/tests/reference/$name.yml" > "$fixture_dir/$name.yml"
done
mkdir "$fixture_dir/result"
uv tool run --from ansible-core==2.21.0 ansible-playbook \
    -i "$fixture_dir/inventory" "$fixture_dir/template.yml" > "$fixture_dir/reference.log" 2>&1
mv "$fixture_dir/result" "$fixture_dir/reference"
mkdir "$fixture_dir/result"
"$binary" playbook -i "$fixture_dir/inventory" "$fixture_dir/template.yml" > "$fixture_dir/rustsible.log" 2>&1
diff -r "$fixture_dir/reference" "$fixture_dir/result"
if uv tool run --from ansible-core==2.21.0 ansible-playbook \
    -i "$fixture_dir/inventory" "$fixture_dir/non_boolean.yml" > "$fixture_dir/reference-condition.log" 2>&1; then
    echo 'Reference unexpectedly accepted a string condition' >&2; exit 1
fi
if "$binary" playbook -i "$fixture_dir/inventory" "$fixture_dir/non_boolean.yml" > "$fixture_dir/rustsible-condition.log" 2>&1; then
    echo 'Rustsible unexpectedly accepted a string condition' >&2; exit 1
fi
[[ ! -e $fixture_dir/result/unsafe-condition ]]
printf 'Ansible 2.21.0 comparison passed: string types, defaults/omit, loops, conditions, template source, literal runtime output\n'
