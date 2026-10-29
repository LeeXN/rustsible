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
for name in template non_boolean files; do
    sed "s|@OUTPUT@|$fixture_dir/result|g" "$repo_dir/tests/reference/$name.yml" > "$fixture_dir/$name.yml"
done
mkdir "$fixture_dir/result"
uv tool run --from ansible-core==2.21.0 ansible-playbook \
    -i "$fixture_dir/inventory" "$fixture_dir/template.yml" > "$fixture_dir/reference.log" 2>&1
mv "$fixture_dir/result" "$fixture_dir/reference"
mkdir "$fixture_dir/result"
"$binary" playbook -i "$fixture_dir/inventory" "$fixture_dir/template.yml" > "$fixture_dir/rustsible.log" 2>&1
diff -r "$fixture_dir/reference" "$fixture_dir/result"
# Compare native file contents, links and permissions against the same pinned
# reference. Unique backup names/timestamps are covered by scenario assertions.
mkdir "$fixture_dir/file-output"
sed "s|@OUTPUT@|$fixture_dir/file-output|g" "$repo_dir/tests/reference/files.yml" > "$fixture_dir/files.yml"
uv tool run --from ansible-core==2.21.0 ansible-playbook -i "$fixture_dir/inventory" "$fixture_dir/files.yml" > "$fixture_dir/reference-files.log" 2>&1
(cd "$fixture_dir/file-output" && find . -exec stat -c '%n:%a:%u:%g' {} \; | sort) > "$fixture_dir/reference-metadata"
mv "$fixture_dir/file-output" "$fixture_dir/reference-files"
mkdir "$fixture_dir/file-output"
"$binary" playbook -i "$fixture_dir/inventory" "$fixture_dir/files.yml" > "$fixture_dir/native-files.log" 2>&1
(cd "$fixture_dir/file-output" && find . -exec stat -c '%n:%a:%u:%g' {} \; | sort) > "$fixture_dir/native-metadata"
diff -u "$fixture_dir/reference-metadata" "$fixture_dir/native-metadata"
diff -r "$fixture_dir/reference-files" "$fixture_dir/file-output"
mkdir "$fixture_dir/execution-output"
sed "s|@OUTPUT@|$fixture_dir/execution-output|g" "$repo_dir/tests/reference/execution.yml" > "$fixture_dir/execution.yml"
uv tool run --from ansible-core==2.21.0 ansible-playbook -i "$fixture_dir/inventory" "$fixture_dir/execution.yml" > "$fixture_dir/reference-execution.log" 2>&1
mv "$fixture_dir/execution-output" "$fixture_dir/reference-execution"
mkdir "$fixture_dir/execution-output"
"$binary" playbook -i "$fixture_dir/inventory" "$fixture_dir/execution.yml" > "$fixture_dir/native-execution.log" 2>&1
diff -r "$fixture_dir/reference-execution" "$fixture_dir/execution-output"
[[ ! -e $fixture_dir/execution-output/should-not-exist ]]
mkdir "$fixture_dir/facts-output"
sed "s|@OUTPUT@|$fixture_dir/facts-output|g" "$repo_dir/tests/reference/facts.yml" > "$fixture_dir/facts.yml"
uv tool run --from ansible-core==2.21.0 ansible-playbook -i "$fixture_dir/inventory" "$fixture_dir/facts.yml" > "$fixture_dir/reference-facts.log" 2>&1
mv "$fixture_dir/facts-output" "$fixture_dir/reference-facts"
mkdir "$fixture_dir/facts-output"
"$binary" playbook -i "$fixture_dir/inventory" "$fixture_dir/facts.yml" > "$fixture_dir/native-facts.log" 2>&1
diff -r "$fixture_dir/reference-facts" "$fixture_dir/facts-output"
cp "$repo_dir/tests/reference/variables-data.yml" "$fixture_dir/variables-data.yml"
mkdir "$fixture_dir/variables-output"
sed "s|@OUTPUT@|$fixture_dir/variables-output|g" "$repo_dir/tests/reference/variables.yml" > "$fixture_dir/variables.yml"
uv tool run --from ansible-core==2.21.0 ansible-playbook -i "$fixture_dir/inventory" "$fixture_dir/variables.yml" -e configuration=extra -e item=extra -e probe=extra > "$fixture_dir/reference-variables.log" 2>&1
mv "$fixture_dir/variables-output" "$fixture_dir/reference-variables"
mkdir "$fixture_dir/variables-output"
"$binary" playbook -i "$fixture_dir/inventory" "$fixture_dir/variables.yml" -e configuration=extra -e item=extra -e probe=extra > "$fixture_dir/native-variables.log" 2>&1
diff -r "$fixture_dir/reference-variables" "$fixture_dir/variables-output"
cp -a "$repo_dir/tests/reference/reuse" "$fixture_dir/reuse"
mkdir "$fixture_dir/reuse-output"
while IFS= read -r -d '' file; do
    sed -i "s|@OUTPUT@|$fixture_dir/reuse-output|g" "$file"
done < <(find "$fixture_dir/reuse" -type f -print0)
ANSIBLE_CONFIG="$fixture_dir/reuse/ansible.cfg" uv tool run --from ansible-core==2.21.0 ansible-playbook -i "$fixture_dir/inventory" "$fixture_dir/reuse/main.yml" > "$fixture_dir/reference-reuse.log" 2>&1
mv "$fixture_dir/reuse-output" "$fixture_dir/reference-reuse"
mkdir "$fixture_dir/reuse-output"
"$binary" --config "$fixture_dir/reuse/ansible.cfg" playbook -i "$fixture_dir/inventory" "$fixture_dir/reuse/main.yml" > "$fixture_dir/native-reuse.log" 2>&1
diff -r "$fixture_dir/reference-reuse" "$fixture_dir/reuse-output"
[[ ! -e $fixture_dir/reuse-output/must-not-run ]]
if uv tool run --from ansible-core==2.21.0 ansible-playbook \
    -i "$fixture_dir/inventory" "$fixture_dir/non_boolean.yml" > "$fixture_dir/reference-condition.log" 2>&1; then
    echo 'Reference unexpectedly accepted a string condition' >&2; exit 1
fi
if "$binary" playbook -i "$fixture_dir/inventory" "$fixture_dir/non_boolean.yml" > "$fixture_dir/rustsible-condition.log" 2>&1; then
    echo 'Rustsible unexpectedly accepted a string condition' >&2; exit 1
fi
[[ ! -e $fixture_dir/result/unsafe-condition ]]
printf 'Ansible 2.21.0 comparison passed: templates/types, native files, retries/conditions, environment, command prediction, fact names/fields/filtering, variable sources, host scope, imports/roles and handler/rescue phases\n'
