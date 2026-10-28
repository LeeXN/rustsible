#!/usr/bin/env bash
set -euo pipefail

repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
binary=$(realpath -- "${1:-$repo_dir/target/debug/rustsible}")
fixture_dir=$(mktemp -d)
container=''
cleanup() {
    local result=$?
    if (( result != 0 )); then
        for log in "$fixture_dir"/*.log; do
            [[ ! -f $log ]] || cat "$log" >&2
        done
    fi
    if [[ -n $container ]] && ! docker rm -f "$container" >/dev/null; then result=1; fi
    rm -rf -- "$fixture_dir"
    exit "$result"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

docker build -q -t rustsible-ssh-fixture:local "$repo_dir/tests/ssh" >/dev/null
ssh-keygen -q -t ed25519 -N '' -f "$fixture_dir/key"
container=$(docker run -d --rm -p 127.0.0.1::22 rustsible-ssh-fixture:local)
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
        content: '{{ identity.stdout | trim }}:{{ ansible_system }}'
        dest: /srv/rustsible/{{ inventory_hostname }}/identity
EOF
snapshot() {
    docker exec "$container" sh -eu -c '
        find /srv/rustsible -type f -exec sha256sum {} \;
        find /srv/rustsible -exec stat -c "%n:%a:%u:%g" {} \;
    '
}
snapshot > "$fixture_dir/before"
"$binary" playbook "$fixture_dir/playbook.yml" -i "$fixture_dir/inventory" --check > "$fixture_dir/check.log"
snapshot > "$fixture_dir/after"
diff -u "$fixture_dir/before" "$fixture_dir/after"
"$binary" playbook "$fixture_dir/playbook.yml" -i "$fixture_dir/inventory" > "$fixture_dir/first.log"
"$binary" playbook "$fixture_dir/playbook.yml" -i "$fixture_dir/inventory" > "$fixture_dir/second.log"
# `id -un` is the sole command task and reports changed on each real execution.
[[ $(grep -c 'ok=6 changed=1 unreachable=0 failed=0' "$fixture_dir/second.log") == 3 ]]
docker exec "$container" sh -eu -c '
    for pair in plain:deploy root:root app:app; do
        name=${pair%:*}; user=${pair#*:}; base=/srv/rustsible/$name
        [ "$(cat "$base/executable")" = replacement ]
        [ "$(cat "$base/identity")" = "$user:Linux" ]
        [ "$(cat "$base/download")" = original ]
        grep -qx appended "$base/lines"
        for file in executable lines download; do
            [ "$(stat -c %a "$base/$file")" = 755 ]
            [ "$(stat -c %U "$base/$file")" = "$user" ]
        done
    done
    [ -z "$(find /srv/rustsible -name ".rustsible-*")" ]
'
printf 'SSH acceptance passed: deploy, root become, app become; check, idempotency, facts/register, bytes/mode/owner, cleanup\n'
