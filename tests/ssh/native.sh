# Sourced by run.sh after setting up a disposable native-architecture SSH host.
docker exec "$container" sh -eu -c '
    ! command -v python
    ! command -v python3
    ! command -v curl
'
cat > "$fixture_dir/native.yml" <<EOF
- hosts: all
  vars:
    ansible_host: 127.0.0.1
    rustsible_remote_tmp: /srv/rustsible/{{ inventory_hostname }}
    expected_user: {plain: deploy, root: root, app: app}
  tasks:
    - command: id -un
      register: identity
    - command: test '{{ identity.stdout | trim }}' = '{{ expected_user[inventory_hostname] }}'
    - command: test '{{ ansible_architecture }}' = '$target_arch'
    - stat:
        path: /etc/passwd
        checksum_algorithm: sha256
      register: metadata
    - command: test '{{ metadata.stat.checksum | length }}' = 64
    - setup: {}
EOF
native_snapshot() {
    docker exec "$container" sh -eu -c '
        find /srv/rustsible -type f -exec sha256sum {} \;
        find /srv/rustsible -exec stat -c "%n:%a:%u:%g" {} \;
    '
}
native_snapshot > "$fixture_dir/before"
rustsible playbook "$fixture_dir/native.yml" -i "$fixture_dir/inventory" --check > "$fixture_dir/native-check.log"
native_snapshot > "$fixture_dir/after"
diff -u "$fixture_dir/before" "$fixture_dir/after"
rustsible playbook "$fixture_dir/native.yml" -i "$fixture_dir/inventory" > "$fixture_dir/native-first.log"
[[ $(grep -c 'ok=7 changed=4 unreachable=0 failed=0' "$fixture_dir/native-first.log") == 3 ]]
docker exec "$container" sh -eu -c '[ -z "$(find /srv/rustsible /tmp -name ".rustsible-worker-*")" ]'
# Required sudo password: the same parent shell validates the credential and
# launches the target identity; neither program arguments nor files carry it.
printf 'deploy:native-test-password\n' | docker exec -i "$container" chpasswd
docker exec "$container" sh -eu -c '
    printf "deploy ALL=(ALL) ALL\n" > /etc/sudoers.d/deploy
    chmod 0440 /etc/sudoers.d/deploy
'
printf '\n[all:vars]\nansible_become_password=native-test-password\n' >> "$fixture_dir/inventory"
chmod 0600 "$fixture_dir/inventory"
rustsible playbook "$fixture_dir/native.yml" -i "$fixture_dir/inventory" > "$fixture_dir/native-password.log"
[[ $(grep -c 'ok=7 changed=4 unreachable=0 failed=0' "$fixture_dir/native-password.log") == 3 ]]
cat > "$fixture_dir/noexec.yml" <<'EOF'
- hosts: plain
  vars:
    ansible_host: 127.0.0.1
    rustsible_remote_tmp: /noexec
  tasks: []
EOF
if rustsible playbook "$fixture_dir/noexec.yml" -i "$fixture_dir/inventory" > "$fixture_dir/native-noexec.log" 2>&1; then
    echo 'noexec worker directory unexpectedly executed a worker' >&2; exit 1
fi
grep -q 'Native worker bootstrap failed' "$fixture_dir/native-noexec.log"
docker exec "$container" sh -eu -c '[ -z "$(ls -A /noexec)" ]'
cat > "$fixture_dir/cancel.yml" <<'EOF'
- hosts: plain
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
    rustsible_remote_tmp: /srv/rustsible/plain
  tasks:
    - shell: echo $$ > /srv/rustsible/plain/cancel-pid; setsid sh -c 'echo $$ > /srv/rustsible/plain/escaped-pid; sleep 60' & wait
EOF
"$controller_binary" --worker-dir "$worker_dir" playbook "$fixture_dir/cancel.yml" -i "$fixture_dir/inventory" > "$fixture_dir/native-cancel.log" 2>&1 &
controller_pid=$!
for _ in {1..100}; do
    if docker exec "$container" sh -c 'test -s /srv/rustsible/plain/escaped-pid'; then break; fi
    sleep 0.05
done
docker exec "$container" sh -eu -c 'test -s /srv/rustsible/plain/cancel-pid; test -s /srv/rustsible/plain/escaped-pid'
kill -TERM "$controller_pid"
if wait "$controller_pid"; then echo 'Terminated controller unexpectedly succeeded' >&2; exit 1; fi
for _ in {1..100}; do
    if docker exec "$container" sh -c '
        [ -z "$(find /srv/rustsible /tmp -name ".rustsible-worker-*")" ] &&
        [ ! -e "/proc/$(cat /srv/rustsible/plain/cancel-pid)" ] &&
        [ ! -e "/proc/$(cat /srv/rustsible/plain/escaped-pid)" ]
    '; then break; fi
    sleep 0.05
done
docker exec "$container" sh -eu -c '
    for marker in /srv/rustsible/plain/cancel-pid /srv/rustsible/plain/escaped-pid; do
        pid=$(cat "$marker")
        if [ -e "/proc/$pid" ]; then
            echo "Business descendant $pid survived controller cancellation" >&2
            cat "/proc/$pid/status" >&2
            exit 1
        fi
    done
    [ -z "$(find /srv/rustsible /tmp -name ".rustsible-worker-*")" ]
'
if [[ $libc == musl ]]; then
    # The shipped static controller also runs inside musl without Python/curl.
    docker exec "$container" mkdir -p /opt/rustsible
    docker cp "$controller_binary" "$container:/opt/rustsible/rustsible" >/dev/null
    docker cp "$worker_dir" "$container:/opt/rustsible/workers" >/dev/null
    docker exec "$container" chmod 0755 /opt/rustsible/rustsible
    docker exec "$container" sh -eu -c '
        printf "localhost ansible_connection=local\n" > /opt/rustsible/inventory
        /opt/rustsible/rustsible ad-hoc localhost -m setup -i /opt/rustsible/inventory
        /opt/rustsible/rustsible ad-hoc localhost -m stat -a "path=/etc/passwd checksum_algorithm=sha256" -i /opt/rustsible/inventory
        [ -z "$(find /tmp -name ".rustsible-worker-*")" ]
    ' > "$fixture_dir/native-musl-controller.log"
fi
printf 'Native SSH acceptance passed (%s -> %s): no Python/curl, setup/stat/command/shell, three identities, password sudo, check invariance, noexec, controller cancellation and setsid descendant cleanup\n' "$controller_arch" "$target_arch"
