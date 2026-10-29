# One controller run reuses one authenticated channel until an explicit reset
# or effective connection change. Count sshd's real successful authentications.
cat > "$fixture_dir/sessions.yml" <<'EOF'
- hosts: root
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - command: /bin/true
      loop: [1, 2, 3, 4, 5, 6, 7, 8]
    - meta: reset_connection
    - command: /bin/true
    - command: /bin/sh -c 'test -z "$(find /tmp -maxdepth 1 -type d -name .rustsible-worker-*)"'
      vars:
        rustsible_remote_tmp: /srv/rustsible/root
    - command: /bin/sh -c 'test -z "$(find /srv/rustsible/root -maxdepth 1 -type d -name .rustsible-worker-*)"'
    - debug:
        msg: '{{ secret }}'
      vars:
        secret: SESSION_NO_LOG_SENTINEL
      no_log: true
EOF
rustsible playbook "$fixture_dir/sessions.yml" -i "$fixture_dir/inventory" > "$fixture_dir/sessions.log"
docker logs "$container" > "$fixture_dir/sshd.log" 2>&1
accepted=$(grep -c 'Accepted publickey for deploy' "$fixture_dir/sshd.log")
[[ $accepted == 4 ]] || { echo "Expected 4 SSH handshakes across reuse/reset/config changes, observed $accepted" >&2; exit 1; }
! grep -q SESSION_NO_LOG_SENTINEL "$fixture_dir/sessions.log"
docker exec "$container" sh -eu -c '
    [ -z "$(find /tmp /srv/rustsible/root -type d -name ".rustsible-worker-*")" ]
'
printf 'SSH session acceptance passed: 12 target calls, 4 authenticated sessions, reset, effective configuration changes, no_log and cleanup\n'
