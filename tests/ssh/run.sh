#!/usr/bin/env bash
set -euo pipefail

repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
controller_binary=$(realpath -- "${1:-$repo_dir/target/debug/rustsible}")
worker_dir=$(realpath -- "${2:-$(dirname -- "$controller_binary")/workers}")
rustsible() { "$controller_binary" --worker-dir "$worker_dir" "$@"; }
mode=${3:-full}
libc=${4:-glibc}
http_fixture=${5:-}
if [[ ( $mode == native || $mode == systems ) && ! -x $http_fixture ]]; then
    echo 'Native acceptance requires the static http_fixture example as argument 5' >&2; exit 1
fi
fixture_dir=$(mktemp -d)
chmod 0700 "$fixture_dir"
container=''
additional_containers=()
cleanup() {
    local result=$?
    if (( result != 0 )); then
        for log in "$fixture_dir"/*.log; do
            [[ ! -f $log ]] || cat "$log" >&2
        done
    fi
    for owned_container in "$container" "${additional_containers[@]}"; do
        if [[ -n $owned_container ]] && ! docker rm -f "$owned_container" >/dev/null; then result=1; fi
    done
    rm -rf -- "$fixture_dir"
    exit "$result"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

fixture_dockerfile=$repo_dir/tests/ssh/Dockerfile
system_flags=()
if [[ $mode == systems ]]; then
    fixture_dockerfile=$repo_dir/tests/systems/Dockerfile
    system_flags=(--privileged --cgroupns=private --tmpfs /run --tmpfs /run/lock)
fi
if [[ $mode == systems && $libc == sysv ]]; then fixture_dockerfile=$repo_dir/tests/ssh/Dockerfile; system_flags=(); fi
if [[ $libc == musl ]]; then
    fixture_dockerfile=$repo_dir/tests/ssh/Dockerfile.musl
    if [[ $mode == systems ]]; then fixture_dockerfile=$repo_dir/tests/systems/Dockerfile.alpine; system_flags=(); fi
fi
build_args=()
if [[ -n ${CODEX_PROXY_CERT:-} ]]; then build_args+=(--secret "id=proxy_ca,src=$CODEX_PROXY_CERT"); fi
docker build -q -f "$fixture_dockerfile" "${build_args[@]}" -t "rustsible-ssh-fixture:$mode-$libc" "$repo_dir/tests/ssh" >/dev/null
ssh-keygen -q -t ed25519 -N '' -f "$fixture_dir/key"
input_tmp_size=$(( $(stat -c %s "$worker_dir/rustsible-worker-linux-$(uname -m)") + 8 * 1024 * 1024 ))
container=$(docker run -d --rm "${system_flags[@]}" -p 127.0.0.1::22 --tmpfs /noexec:rw,noexec,mode=1777 --tmpfs /small:rw,size=64k,mode=1777 \
    --tmpfs "/input-small:rw,exec,size=$input_tmp_size,mode=1777" --tmpfs "/input-inodes:rw,exec,size=$input_tmp_size,nr_inodes=3,mode=1777" \
    -e HTTP_PROXY=http://127.0.0.1:18080 -e http_proxy=http://127.0.0.1:18080 \
    -e HTTPS_PROXY=http://127.0.0.1:18080 -e https_proxy=http://127.0.0.1:18080 \
    -e NO_PROXY=localhost,127.0.0.1 -e no_proxy=localhost,127.0.0.1 \
    "rustsible-ssh-fixture:$mode-$libc")
docker cp "$fixture_dir/key.pub" "$container:/tmp/authorized_keys" >/dev/null
docker exec "$container" sh -eu -c '
    install -d -m 0700 -o deploy -g deploy /home/deploy/.ssh
    install -m 0600 -o deploy -g deploy /tmp/authorized_keys /home/deploy/.ssh/authorized_keys
    for pair in plain:deploy root:root app:app; do
        name=${pair%:*}; user=${pair#*:}
        install -d -m 0755 -o "$user" -g "$user" "/srv/rustsible/$name"
        for file in executable lines download; do
            printf original > "/srv/rustsible/$name/$file"
            chmod 0755 "/srv/rustsible/$name/$file"
            chown "$user:$user" "/srv/rustsible/$name/$file"
        done
    done
'
controller_arch=$(uname -m)
target_arch=$(docker exec "$container" uname -m)
case "$controller_arch" in x86_64|aarch64) ;; *) echo 'Unsupported acceptance architecture' >&2; exit 1;; esac
[[ $target_arch == "$controller_arch" ]] || { echo 'SSH fixture must run on the native architecture' >&2; exit 1; }
port=$(docker port "$container" 22/tcp)
port=${port##*:}
for _ in {1..100}; do
    if docker exec "$container" test -f /run/sshd.pid; then break; fi
    sleep 0.1
done
docker exec "$container" sh -c 'cat /etc/ssh/ssh_host_*_key.pub' |
    while IFS= read -r public_key; do
        printf '[127.0.0.1]:%s %s\n' "$port" "$public_key"
    done > "$fixture_dir/known_hosts"
cat > "$fixture_dir/inventory" <<EOF
[all]
plain ansible_become=false
root ansible_become=true ansible_become_user=root
app ansible_become=true ansible_become_user=app
[all:vars]
ansible_host=unreachable.invalid
ansible_connection=ssh
fixture_port=$port
ansible_port="{{ fixture_port }}"
ansible_user=deploy
ansible_ssh_private_key_file=$fixture_dir/key
rustsible_known_hosts_file=$fixture_dir/known_hosts
EOF
if [[ $mode == systems ]]; then
    if [[ $libc == musl ]]; then source "$repo_dir/tests/systems/alpine.sh"; elif [[ $libc == sysv ]]; then source "$repo_dir/tests/systems/sysv.sh"; else source "$repo_dir/tests/systems/cases.sh"; fi
    exit 0
fi
if [[ $mode == native ]]; then
    source "$repo_dir/tests/ssh/native.sh"
    source "$repo_dir/tests/ssh/files.sh"
    exit 0
fi
if [[ $mode == sessions ]]; then
    source "$repo_dir/tests/ssh/sessions.sh"
    exit 0
fi
if [[ $mode == controls ]]; then
    source "$repo_dir/tests/ssh/controls.sh"
    exit 0
fi
if [[ $mode == variables ]]; then
    source "$repo_dir/tests/ssh/variables.sh"
    exit 0
fi
if [[ $mode == reuse ]]; then
    source "$repo_dir/tests/ssh/reuse.sh"
    exit 0
fi
if [[ $mode == strategy ]]; then
    source "$repo_dir/tests/ssh/strategy.sh"
    exit 0
fi
if [[ $mode == async ]]; then
    source "$repo_dir/tests/ssh/async.sh"
    exit 0
fi
if [[ $mode == basic ]]; then
    source "$repo_dir/tests/ssh/basic.sh"
    exit 0
fi
cat > "$fixture_dir/playbook.yml" <<'EOF'
- hosts: all
  become: false
  become_user: unused-keyword-user
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - command: id -un
      register: identity
    - copy:
        content: replacement
        dest: /srv/rustsible/{{ inventory_hostname }}/executable
    - lineinfile:
        path: /srv/rustsible/{{ inventory_hostname }}/lines
        line: appended
    - get_url:
        url: http://127.0.0.1:9/must-not-be-contacted
        dest: /srv/rustsible/{{ inventory_hostname }}/download
        force: false
- hosts: all
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - copy:
        content: '{{ identity.stdout | trim }}:{{ ansible_system }}:{{ ansible_architecture }}'
        dest: /srv/rustsible/{{ inventory_hostname }}/identity
EOF
snapshot() {
    docker exec "$container" sh -eu -c '
        find /srv/rustsible -type f -exec sha256sum {} \;
        find /srv/rustsible -exec stat -c "%n:%a:%u:%g" {} \;
    '
}
snapshot > "$fixture_dir/before"
rustsible playbook "$fixture_dir/playbook.yml" -i "$fixture_dir/inventory" --check > "$fixture_dir/check.log"
snapshot > "$fixture_dir/after"
diff -u "$fixture_dir/before" "$fixture_dir/after"
rustsible playbook "$fixture_dir/playbook.yml" -i "$fixture_dir/inventory" > "$fixture_dir/first.log"
rustsible playbook "$fixture_dir/playbook.yml" -i "$fixture_dir/inventory" > "$fixture_dir/second.log"
# `id -un` is the sole command task and reports changed on each real execution.
[[ $(grep -c 'ok=6 changed=1 unreachable=0 failed=0' "$fixture_dir/second.log") == 3 ]]
cat > "$fixture_dir/unsafe-parent.yml" <<'EOF'
- hosts: root
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - copy:
        content: must-not-change
        dest: /srv/rustsible/app/executable
EOF
if rustsible playbook "$fixture_dir/unsafe-parent.yml" -i "$fixture_dir/inventory" > "$fixture_dir/unsafe-parent.log" 2>&1; then
    echo 'Unsafe cross-owner staging unexpectedly succeeded' >&2; exit 1
fi
grep -q 'Unsafe destination directory owner' "$fixture_dir/unsafe-parent.log"
docker exec "$container" sh -eu -c '
    expected_arch=$(uname -m)
    for pair in plain:deploy root:root app:app; do
        name=${pair%:*}; user=${pair#*:}; base=/srv/rustsible/$name
        [ "$(cat "$base/executable")" = replacement ]
        [ "$(cat "$base/identity")" = "$user:Linux:$expected_arch" ]
        [ "$(cat "$base/download")" = original ]
        grep -qx appended "$base/lines"
        for file in executable lines download; do
            [ "$(stat -c %a "$base/$file")" = 755 ]
            [ "$(stat -c %U "$base/$file")" = "$user" ]
        done
    done
    [ -z "$(find /srv/rustsible -name ".rustsible-*")" ]
'
cat > "$fixture_dir/interrupted.yml" <<'EOF'
- hosts: plain
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
    rustsible_command_timeout: 1
  tasks:
    - shell: printf x >> /srv/rustsible/plain/interrupted; sleep 3
      ignore_errors: true
- hosts: plain
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - copy:
        content: must-not-run
        dest: /srv/rustsible/plain/after-interruption
EOF
if rustsible playbook "$fixture_dir/interrupted.yml" -i "$fixture_dir/inventory" > "$fixture_dir/interrupted.log" 2>&1; then
    echo 'Interrupted mutation unexpectedly succeeded' >&2; exit 1
fi
grep -q 'uncertain=1' "$fixture_dir/interrupted.log"
docker exec "$container" sh -eu -c '
    [ "$(cat /srv/rustsible/plain/interrupted)" = x ]
    [ ! -e /srv/rustsible/plain/after-interruption ]
'
printf 'SSH acceptance passed (%s -> %s): deploy, root become, app become; check, idempotency, facts/register, bytes/mode/owner, cleanup, uncertain/no replay\n' "$controller_arch" "$target_arch"
