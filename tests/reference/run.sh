#!/usr/bin/env bash
set -euo pipefail
repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
binary=$(realpath -- "${1:-$repo_dir/target/debug/rustsible}")
network_fixture=$(realpath -- "${2:-$(dirname -- "$binary")/examples/http_fixture}")
fixture_pid=''
fixture_dir=$(mktemp -d /tmp/rustsible-reference.XXXXXXXX)
cleanup() {
    local result=$?
    if [[ -n $fixture_pid ]]; then kill -KILL "$fixture_pid" 2>/dev/null || true; wait "$fixture_pid" 2>/dev/null || true; fi
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
printf '[rollout]\na ansible_connection=local\nb ansible_connection=local\nc ansible_connection=local\nd ansible_connection=local\n[delegation]\norigin ansible_connection=ssh ansible_host=unreachable.invalid marker=origin\ntarget ansible_connection=local marker=target\n' > "$fixture_dir/strategy-inventory"
mkdir "$fixture_dir/strategy-output"
sed "s|@OUTPUT@|$fixture_dir/strategy-output|g" "$repo_dir/tests/reference/strategy.yml" > "$fixture_dir/strategy.yml"
uv tool run --from ansible-core==2.21.0 ansible-playbook -i "$fixture_dir/strategy-inventory" "$fixture_dir/strategy.yml" > "$fixture_dir/reference-strategy.log" 2>&1
mv "$fixture_dir/strategy-output" "$fixture_dir/reference-strategy"
mkdir "$fixture_dir/strategy-output"
"$binary" playbook -i "$fixture_dir/strategy-inventory" "$fixture_dir/strategy.yml" > "$fixture_dir/native-strategy.log" 2>&1
diff -r "$fixture_dir/reference-strategy" "$fixture_dir/strategy-output"
for percentage in 49 50; do
    mkdir "$fixture_dir/limit-$percentage"
    sed "s|@OUTPUT@|$fixture_dir/limit-$percentage|g; s|@PERCENTAGE@|$percentage|g" "$repo_dir/tests/reference/failure-limit.yml" > "$fixture_dir/limit-$percentage.yml"
    if uv tool run --from ansible-core==2.21.0 ansible-playbook -i "$fixture_dir/strategy-inventory" "$fixture_dir/limit-$percentage.yml" > "$fixture_dir/reference-limit-$percentage.log" 2>&1; then
        echo 'Reference unexpectedly accepted a failed rollout' >&2; exit 1
    fi
    mv "$fixture_dir/limit-$percentage" "$fixture_dir/reference-limit-$percentage"
    mkdir "$fixture_dir/limit-$percentage"
    if "$binary" playbook -i "$fixture_dir/strategy-inventory" "$fixture_dir/limit-$percentage.yml" > "$fixture_dir/native-limit-$percentage.log" 2>&1; then
        echo 'Rustsible unexpectedly accepted a failed rollout' >&2; exit 1
    fi
    diff -r "$fixture_dir/reference-limit-$percentage" "$fixture_dir/limit-$percentage"
done
mkdir "$fixture_dir/fatal-output"
sed "s|@OUTPUT@|$fixture_dir/fatal-output|g" "$repo_dir/tests/reference/fatal-rescue.yml" > "$fixture_dir/fatal.yml"
if uv tool run --from ansible-core==2.21.0 ansible-playbook -i "$fixture_dir/strategy-inventory" "$fixture_dir/fatal.yml" > "$fixture_dir/reference-fatal.log" 2>&1; then
    echo 'Ansible 2.21 unexpectedly cleared the fatal failure exit status' >&2; exit 1
fi
[[ $(cat "$fixture_dir/fatal-output/continued-a") == continued && $(cat "$fixture_dir/fatal-output/later-a") == later-play ]]
[[ ! -e $fixture_dir/fatal-output/later-d ]]
mv "$fixture_dir/fatal-output" "$fixture_dir/reference-fatal"
mkdir "$fixture_dir/fatal-output"
if "$binary" playbook -i "$fixture_dir/strategy-inventory" "$fixture_dir/fatal.yml" > "$fixture_dir/native-fatal.log" 2>&1; then
    echo 'Rustsible unexpectedly cleared the fatal failure exit status' >&2; exit 1
fi
diff -r "$fixture_dir/reference-fatal" "$fixture_dir/fatal-output"
mkdir "$fixture_dir/previous-output"
sed "s|@OUTPUT@|$fixture_dir/previous-output|g" "$repo_dir/tests/reference/previous-failures.yml" > "$fixture_dir/previous.yml"
if uv tool run --from ansible-core==2.21.0 ansible-playbook -i "$fixture_dir/strategy-inventory" "$fixture_dir/previous.yml" > "$fixture_dir/reference-previous.log" 2>&1; then
    echo 'Reference unexpectedly cleared earlier host failures' >&2; exit 1
fi
[[ $(cat "$fixture_dir/previous-output/batch-c") == c,d && $(cat "$fixture_dir/previous-output/survived-d") == survived ]]
mv "$fixture_dir/previous-output" "$fixture_dir/reference-previous"
mkdir "$fixture_dir/previous-output"
if "$binary" playbook -i "$fixture_dir/strategy-inventory" "$fixture_dir/previous.yml" > "$fixture_dir/native-previous.log" 2>&1; then
    echo 'Rustsible unexpectedly cleared earlier host failures' >&2; exit 1
fi
diff -r "$fixture_dir/reference-previous" "$fixture_dir/previous-output"
mkdir -p "$fixture_dir/roles" "$fixture_dir/free-output"
cp -a "$repo_dir/tests/reference/free-role" "$fixture_dir/roles/demo"
sed "s|@OUTPUT@|$fixture_dir/free-output|g" "$repo_dir/tests/reference/free.yml" > "$fixture_dir/free.yml"
uv tool run --from ansible-core==2.21.0 ansible-playbook -i "$fixture_dir/strategy-inventory" "$fixture_dir/free.yml" > "$fixture_dir/reference-free.log" 2>&1
mv "$fixture_dir/free-output" "$fixture_dir/reference-free"
mkdir "$fixture_dir/free-output"
"$binary" playbook -i "$fixture_dir/strategy-inventory" "$fixture_dir/free.yml" > "$fixture_dir/native-free.log" 2>&1
diff -r "$fixture_dir/reference-free" "$fixture_dir/free-output"
mkdir "$fixture_dir/async-output"
sed "s|@OUTPUT@|$fixture_dir/async-output|g" "$repo_dir/tests/reference/async.yml" > "$fixture_dir/async.yml"
uv tool run --from ansible-core==2.21.0 ansible-playbook -i "$fixture_dir/inventory" "$fixture_dir/async.yml" > "$fixture_dir/reference-async.log" 2>&1
mv "$fixture_dir/async-output" "$fixture_dir/reference-async"
mkdir "$fixture_dir/async-output"
"$binary" playbook -i "$fixture_dir/inventory" "$fixture_dir/async.yml" > "$fixture_dir/native-async.log" 2>&1
diff -r "$fixture_dir/reference-async" "$fixture_dir/async-output"
mkdir "$fixture_dir/assert-output"
sed "s|@OUTPUT@|$fixture_dir/assert-output|g" "$repo_dir/tests/reference/assertions.yml" > "$fixture_dir/assertions.yml"
uv tool run --from ansible-core==2.21.0 ansible-playbook -i "$fixture_dir/inventory" "$fixture_dir/assertions.yml" > "$fixture_dir/reference-assertions.log" 2>&1
mv "$fixture_dir/assert-output" "$fixture_dir/reference-assertions"
mkdir "$fixture_dir/assert-output"
"$binary" playbook -i "$fixture_dir/inventory" "$fixture_dir/assertions.yml" > "$fixture_dir/native-assertions.log" 2>&1
diff -r "$fixture_dir/reference-assertions" "$fixture_dir/assert-output"
[[ -x $network_fixture ]] || { echo 'Reference comparison requires the http_fixture example executable' >&2; exit 1; }
"$network_fixture" 127.0.0.1:0 "$fixture_dir" > "$fixture_dir/network-address.log" &
fixture_pid=$!
for _ in {1..100}; do [[ ! -s $fixture_dir/network-address.log ]] || break; sleep 0.05; done
address=$(head -n 1 "$fixture_dir/network-address.log")
port=${address##*:}
mkdir "$fixture_dir/network-output"
sed "s|@OUTPUT@|$fixture_dir/network-output|g; s|@ADDRESS@|$address|g; s|@PORT@|$port|g" "$repo_dir/tests/reference/network.yml" > "$fixture_dir/network.yml"
uv tool run --from ansible-core==2.21.0 ansible-playbook -i "$fixture_dir/inventory" "$fixture_dir/network.yml" > "$fixture_dir/reference-network.log" 2>&1
mv "$fixture_dir/network-output" "$fixture_dir/reference-network"
mkdir "$fixture_dir/network-output"
"$binary" playbook -i "$fixture_dir/inventory" "$fixture_dir/network.yml" > "$fixture_dir/native-network.log" 2>&1
diff -r "$fixture_dir/reference-network" "$fixture_dir/network-output"
mkdir "$fixture_dir/artifact-source" "$fixture_dir/archive-content" "$fixture_dir/artifact-output"
printf pinned-version > "$fixture_dir/artifact-source/version"
git -C "$fixture_dir/artifact-source" init -q
git -C "$fixture_dir/artifact-source" add .
git -C "$fixture_dir/artifact-source" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm initial
git -C "$fixture_dir/artifact-source" tag v1
printf archive-data > "$fixture_dir/archive-content/tool"
chmod 0755 "$fixture_dir/archive-content/tool"
tar -czf "$fixture_dir/artifact.tar.gz" -C "$fixture_dir/archive-content" tool
sed "s|@OUTPUT@|$fixture_dir/artifact-output|g; s|@REPO@|$fixture_dir/artifact-source|g; s|@ARCHIVE@|$fixture_dir/artifact.tar.gz|g" "$repo_dir/tests/reference/artifacts.yml" > "$fixture_dir/artifacts.yml"
uv tool run --from ansible-core==2.21.0 ansible-playbook -i "$fixture_dir/inventory" "$fixture_dir/artifacts.yml" > "$fixture_dir/reference-artifacts.log" 2>&1
mv "$fixture_dir/artifact-output" "$fixture_dir/reference-artifacts"
mkdir "$fixture_dir/artifact-output"
"$binary" playbook -i "$fixture_dir/inventory" "$fixture_dir/artifacts.yml" > "$fixture_dir/native-artifacts.log" 2>&1
diff -r --exclude=.git "$fixture_dir/reference-artifacts" "$fixture_dir/artifact-output"
[[ $(stat -c %a "$fixture_dir/artifact-output/files/tool") == 755 ]]
cp -a "$repo_dir/tests/reference/tags" "$fixture_dir/tags"
for selection in alpha never tagged untagged all; do
    mkdir "$fixture_dir/tags-output"
    selection_options=(--tags "$selection")
    if [[ $selection == all ]]; then selection_options+=(--skip-tags alpha); fi
    uv tool run --from ansible-core==2.21.0 ansible-playbook -i "$fixture_dir/inventory" "$fixture_dir/tags/main.yml" -e "out=$fixture_dir/tags-output" "${selection_options[@]}" > "$fixture_dir/reference-tags-$selection.log" 2>&1
    mv "$fixture_dir/tags-output" "$fixture_dir/reference-tags-$selection"
    mkdir "$fixture_dir/tags-output"
    "$binary" playbook -i "$fixture_dir/inventory" "$fixture_dir/tags/main.yml" -e "out=$fixture_dir/tags-output" "${selection_options[@]}" > "$fixture_dir/native-tags-$selection.log" 2>&1
    diff -r "$fixture_dir/reference-tags-$selection" "$fixture_dir/tags-output"
    mv "$fixture_dir/tags-output" "$fixture_dir/native-tags-$selection"
done
mkdir "$fixture_dir/tags-output"
if uv tool run --from ansible-core==2.21.0 ansible-playbook -i "$fixture_dir/inventory" "$fixture_dir/tags/failure.yml" -e "out=$fixture_dir/tags-output" --tags alpha > "$fixture_dir/reference-tags-failure.log" 2>&1; then echo 'Reference incorrectly cleared a tag-filtered rescue failure' >&2; exit 1; fi
mv "$fixture_dir/tags-output" "$fixture_dir/reference-tags-failure"
mkdir "$fixture_dir/tags-output"
if "$binary" playbook -i "$fixture_dir/inventory" "$fixture_dir/tags/failure.yml" -e "out=$fixture_dir/tags-output" --tags alpha > "$fixture_dir/native-tags-failure.log" 2>&1; then echo 'Native executor incorrectly cleared a tag-filtered rescue failure' >&2; exit 1; fi
diff -r "$fixture_dir/reference-tags-failure" "$fixture_dir/tags-output"
[[ ! -e $fixture_dir/tags-output/rescued && ! -e $fixture_dir/tags-output/after ]]
uv tool run --from ansible-core==2.21.0 ansible-playbook -i "$fixture_dir/inventory" "$fixture_dir/tags/import.yml" -e "out=$fixture_dir/tags-output" --tags alpha > "$fixture_dir/reference-import-tags.log" 2>&1
mv "$fixture_dir/tags-output/imported" "$fixture_dir/reference-imported"
"$binary" playbook -i "$fixture_dir/inventory" "$fixture_dir/tags/import.yml" -e "out=$fixture_dir/tags-output" --tags alpha > "$fixture_dir/native-import-tags.log" 2>&1
diff "$fixture_dir/reference-imported" "$fixture_dir/tags-output/imported"
printf 'fixture-password\n' > "$fixture_dir/vault-password"
printf 'second-password\n' > "$fixture_dir/vault-other-password"
chmod 0600 "$fixture_dir/vault-password" "$fixture_dir/vault-other-password"
printf 'literal {{ must_not_execute }}' > "$fixture_dir/vault-literal"
"$binary" vault encrypt "$fixture_dir/vault-literal" --vault-id "prod@$fixture_dir/vault-password" > "$fixture_dir/native-vault-encrypt.log" 2>&1
uv tool run --from ansible-core==2.21.0 ansible-vault decrypt "$fixture_dir/vault-literal" --vault-id "prod@$fixture_dir/vault-password" --output "$fixture_dir/reference-vault-plain" > "$fixture_dir/reference-vault-decrypt.log" 2>&1
printf 'literal {{ must_not_execute }}' > "$fixture_dir/vault-expected"
cmp "$fixture_dir/vault-expected" "$fixture_dir/reference-vault-plain"
{ printf 'literal: !vault |\n'; sed 's/^/  /' "$fixture_dir/vault-literal"; printf 'primary: first\n'; } > "$fixture_dir/vault-vars.yml"
printf 'secondary: second\n' > "$fixture_dir/vault-other.yml"
uv tool run --from ansible-core==2.21.0 ansible-vault encrypt "$fixture_dir/vault-other.yml" --vault-id "dev@$fixture_dir/vault-other-password" > "$fixture_dir/reference-vault-encrypt.log" 2>&1
"$binary" vault decrypt "$fixture_dir/vault-other.yml" --vault-id "dev@$fixture_dir/vault-other-password" --output "$fixture_dir/native-vault-plain" > "$fixture_dir/native-vault-decrypt.log" 2>&1
printf 'secondary: second\n' > "$fixture_dir/vault-other-expected"
cmp "$fixture_dir/vault-other-expected" "$fixture_dir/native-vault-plain"
printf '{{ primary }}|{{ secondary }}|{{ literal }}' > "$fixture_dir/vault-template.j2"
"$binary" vault encrypt "$fixture_dir/vault-template.j2" --vault-password-file "$fixture_dir/vault-password" > "$fixture_dir/native-vault-template.log" 2>&1
mkdir "$fixture_dir/vault-output"
sed "s|@OUTPUT@|$fixture_dir/vault-output|g; s|@VARIABLES@|$fixture_dir/vault-vars.yml|g; s|@OTHER_VARIABLES@|$fixture_dir/vault-other.yml|g; s|@TEMPLATE@|$fixture_dir/vault-template.j2|g" "$repo_dir/tests/reference/vault.yml" > "$fixture_dir/vault.yml"
vault_options=(--vault-id "prod@$fixture_dir/vault-password" --vault-id "dev@$fixture_dir/vault-other-password")
uv tool run --from ansible-core==2.21.0 ansible-playbook -i "$fixture_dir/inventory" "$fixture_dir/vault.yml" "${vault_options[@]}" > "$fixture_dir/reference-vault-playbook.log" 2>&1
mv "$fixture_dir/vault-output" "$fixture_dir/reference-vault-output"
mkdir "$fixture_dir/vault-output"
"$binary" playbook -i "$fixture_dir/inventory" "$fixture_dir/vault.yml" "${vault_options[@]}" --diff -vvv > "$fixture_dir/native-vault-playbook.log" 2>&1
diff -r "$fixture_dir/reference-vault-output" "$fixture_dir/vault-output"
if grep -q 'fixture-password\|literal {{ must_not_execute }}' "$fixture_dir/native-vault-playbook.log"; then echo 'Vault diagnostics leaked a protected value' >&2; exit 1; fi
if uv tool run --from ansible-core==2.21.0 ansible-playbook \
    -i "$fixture_dir/inventory" "$fixture_dir/non_boolean.yml" > "$fixture_dir/reference-condition.log" 2>&1; then
    echo 'Reference unexpectedly accepted a string condition' >&2; exit 1
fi
if "$binary" playbook -i "$fixture_dir/inventory" "$fixture_dir/non_boolean.yml" > "$fixture_dir/rustsible-condition.log" 2>&1; then
    echo 'Rustsible unexpectedly accepted a string condition' >&2; exit 1
fi
[[ ! -e $fixture_dir/result/unsafe-condition ]]
printf 'Ansible 2.21.0 comparison passed: templates/types, native files, retries/conditions, environment, command prediction, facts, variables, imports/roles, handlers/rescue, serial/run_once, failure thresholds, delegation, free strategy native async/poll/status, assertions, target wait/HTTP readiness Git/archive deployment inherited/static/dynamic tags and bidirectional Vault encryption\n'

source "$repo_dir/tests/reference/collections.sh"
printf "Pinned Ansible 2.21 declarative collection comparison passed\n"
