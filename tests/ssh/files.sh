# Sourced after the native SSH cases; every target runs without Python/curl.
docker exec "$container" install -d -m 0755 /srv/http
docker cp "$http_fixture" "$container:/srv/http/server" >/dev/null
docker cp "$repo_dir/tests/fixtures/tls/." "$container:/srv/http/" >/dev/null
docker exec "$container" chmod 0755 /srv/http/server
docker exec -d "$container" sh -c 'exec /srv/http/server 127.0.0.1:18080 /srv/http >/srv/http/http.log 2>&1'
docker exec -d "$container" sh -c 'exec /srv/http/server 127.0.0.1:18081 /srv/http --tls >/srv/http/https.log 2>&1'
for _ in {1..100}; do
    if docker exec "$container" sh -c 'test -s /srv/http/http.log && test -s /srv/http/https.log'; then break; fi
    sleep 0.05
done
docker exec "$container" sh -eu -c '
    test -s /srv/http/http.log; test -s /srv/http/https.log
    printf "PermitUserEnvironment yes\n" >> /etc/ssh/sshd_config
    /usr/sbin/sshd -T | grep -q "permituserenvironment yes"
    printf "HTTP_PROXY=http://127.0.0.1:18080\nhttp_proxy=http://127.0.0.1:18080\nHTTPS_PROXY=http://127.0.0.1:18080\nhttps_proxy=http://127.0.0.1:18080\nNO_PROXY=127.0.0.1,localhost\nno_proxy=127.0.0.1,localhost\n" > /home/deploy/.ssh/environment
    chown deploy:deploy /home/deploy/.ssh/environment
    chmod 0600 /home/deploy/.ssh/environment
    for variable in HTTP_PROXY http_proxy HTTPS_PROXY https_proxy NO_PROXY no_proxy; do
        printf "Defaults env_keep += %s\n" "$variable"
    done > /etc/sudoers.d/http-environment
    visudo -cf /etc/sudoers >/dev/null
    chmod 0440 /etc/sudoers.d/http-environment
    kill -HUP "$(cat /run/sshd.pid)"
'
# sshd re-execs on HUP. Probe only a read-only connection before attributing a
# following failure to TLS rather than that short daemon restart window.
for _ in {1..100}; do
    if ssh -o BatchMode=yes -o StrictHostKeyChecking=yes -o "UserKnownHostsFile=$fixture_dir/known_hosts" -i "$fixture_dir/key" -p "$port" deploy@127.0.0.1 true 2>/dev/null; then break; fi
    sleep 0.05
done
cat > "$fixture_dir/untrusted.yml" <<'EOF'
- hosts: plain
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - get_url:
        url: https://native-user:native-password-canary@127.0.0.1:18081/hello?private-token-canary
        dest: /srv/rustsible/plain/download
        force: true
      no_log: true
EOF
sed 's/no_log: true/no_log: false/' "$fixture_dir/untrusted.yml" > "$fixture_dir/untrusted-readable.yml"
if rustsible playbook "$fixture_dir/untrusted-readable.yml" -i "$fixture_dir/inventory" > "$fixture_dir/untrusted-readable.log" 2>&1; then
    echo 'Unknown HTTPS CA unexpectedly trusted' >&2; exit 1
fi
grep -q 'unreachable=0 failed=1.*uncertain=0' "$fixture_dir/untrusted-readable.log"
grep -qi 'certificate\|UnknownIssuer' "$fixture_dir/untrusted-readable.log"
! grep -q 'native-password-canary\|private-token-canary' "$fixture_dir/untrusted-readable.log"
if rustsible playbook "$fixture_dir/untrusted.yml" -i "$fixture_dir/inventory" -vv > "$fixture_dir/untrusted.log" 2>&1; then
    echo 'Unknown HTTPS CA unexpectedly trusted' >&2; exit 1
fi
grep -q 'unreachable=0 failed=1.*uncertain=0' "$fixture_dir/untrusted.log"
! grep -q 'native-password-canary\|private-token-canary' "$fixture_dir/untrusted.log"
docker exec "$container" sh -eu -c '
    [ "$(cat /srv/rustsible/plain/download)" = original ]
    install -m 0644 /srv/http/ca.pem /usr/local/share/ca-certificates/rustsible-test.crt
    update-ca-certificates >/dev/null
'
# Binary input exceeds the old 16 MiB command-output cap and includes bytes
# that cannot pass through a UTF-8/JSON content conversion.
dd if=/dev/zero of="$fixture_dir/large.bin" bs=1048576 count=20 status=none
printf '\377\000\200\012' >> "$fixture_dir/large.bin"
large_checksum=$(sha256sum "$fixture_dir/large.bin")
large_checksum=${large_checksum%% *}
docker cp "$fixture_dir/large.bin" "$container:/srv/http/large.bin" >/dev/null
docker exec "$container" sh -eu -c '
    for pair in plain:deploy root:root app:app; do
        name=${pair%:*}; user=${pair#*:}
        for file in streamed downloaded https proxy; do
            printf original > "/srv/rustsible/$name/$file"
            chmod 0755 "/srv/rustsible/$name/$file"
            chown "$user:$user" "/srv/rustsible/$name/$file"
        done
    done
    printf original > /small/artifact
    chmod 0755 /small/artifact
'
docker exec "$container" sh -eu -c '
    printf original > /srv/rustsible/root/capability
    chmod 0755 /srv/rustsible/root/capability
    setcap cap_net_bind_service=ep /srv/rustsible/root/capability
    getcap /srv/rustsible/root/capability
' > "$fixture_dir/capability-before"
cat > "$fixture_dir/files.yml" <<EOF
- hosts: all
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - copy:
        src: $fixture_dir/large.bin
        dest: /srv/rustsible/{{ inventory_hostname }}/streamed
    - get_url:
        url: http://127.0.0.1:18080/large.bin
        dest: /srv/rustsible/{{ inventory_hostname }}/downloaded
        checksum: sha256:$large_checksum
    - get_url:
        url: https://127.0.0.1:18081/redirect
        dest: /srv/rustsible/{{ inventory_hostname }}/https
        checksum: sha1:aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d
    - get_url:
        url: http://must-not-resolve.invalid/hello
        dest: /srv/rustsible/{{ inventory_hostname }}/proxy
        checksum: sha1:aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d
EOF
cat >> "$fixture_dir/files.yml" <<'EOF'
- hosts: root
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - copy:
        content: replacement-capability
        dest: /srv/rustsible/root/capability
EOF
native_snapshot > "$fixture_dir/files-before"
rustsible playbook "$fixture_dir/files.yml" -i "$fixture_dir/inventory" --check > "$fixture_dir/files-check.log"
native_snapshot > "$fixture_dir/files-after"
diff -u "$fixture_dir/files-before" "$fixture_dir/files-after"
rustsible playbook "$fixture_dir/files.yml" -i "$fixture_dir/inventory" > "$fixture_dir/files-first.log"
rustsible playbook "$fixture_dir/files.yml" -i "$fixture_dir/inventory" > "$fixture_dir/files-second.log"
[[ $(grep -c 'ok=5 changed=0 unreachable=0 failed=0' "$fixture_dir/files-second.log") == 2 ]]
grep -q 'ok=6 changed=0 unreachable=0 failed=0' "$fixture_dir/files-second.log"
docker exec "$container" getcap /srv/rustsible/root/capability > "$fixture_dir/capability-after"
diff -u "$fixture_dir/capability-before" "$fixture_dir/capability-after"
docker exec "$container" sh -eu -c '
    expected=$1
    for pair in plain:deploy root:root app:app; do
        name=${pair%:*}; user=${pair#*:}
        for file in streamed downloaded; do
            checksum=$(sha256sum "/srv/rustsible/$name/$file")
            [ "${checksum%% *}" = "$expected" ]
        done
        for file in https proxy; do [ "$(cat "/srv/rustsible/$name/$file")" = hello ]; done
        for file in streamed downloaded https proxy; do
            [ "$(stat -c %a "/srv/rustsible/$name/$file")" = 755 ]
            [ "$(stat -c %U "/srv/rustsible/$name/$file")" = "$user" ]
        done
    done
    [ -z "$(find /srv/rustsible /tmp -type d -name ".rustsible-*")" ]
' sh "$large_checksum"
cat > "$fixture_dir/disk-full.yml" <<'EOF'
- hosts: root
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - get_url:
        url: http://127.0.0.1:18080/large.bin
        dest: /small/artifact
        force: true
EOF
docker exec "$container" stat -c '%d:%i:%a:%u:%g:%s' /small/artifact > "$fixture_dir/full-before"
if rustsible playbook "$fixture_dir/disk-full.yml" -i "$fixture_dir/inventory" > "$fixture_dir/disk-full.log" 2>&1; then
    echo 'Disk-full update unexpectedly succeeded' >&2; exit 1
fi
grep -qi 'No space left\|StorageFull' "$fixture_dir/disk-full.log"
docker exec "$container" stat -c '%d:%i:%a:%u:%g:%s' /small/artifact > "$fixture_dir/full-after"
diff -u "$fixture_dir/full-before" "$fixture_dir/full-after"
docker exec "$container" sh -eu -c '[ "$(cat /small/artifact)" = original ]; [ "$(find /small -mindepth 1 | wc -l)" = 1 ]'
printf 'Native files/HTTP SSH acceptance passed (%s): no Python/curl; 20 MiB binary stream and download; private CA, proxy, redirects, check invariance, three identities, idempotency and real disk-full preservation/cleanup\n' "$libc"
# Exhaust input staging both before Ready (inodes) and during streaming
# (bytes). These are structured module failures, not unreachable SSH hosts.
for temporary in /input-small /input-inodes; do
    cat > "$fixture_dir/input-full.yml" <<EOF
- hosts: plain
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
    rustsible_remote_tmp: $temporary
  tasks:
    - copy:
        src: $fixture_dir/large.bin
        dest: /srv/rustsible/plain/download
EOF
    if rustsible playbook "$fixture_dir/input-full.yml" -i "$fixture_dir/inventory" > "$fixture_dir/input-full.log" 2>&1; then
        echo 'Full input staging unexpectedly succeeded' >&2; exit 1
    fi
    grep -q 'unreachable=0 failed=1.*uncertain=0' "$fixture_dir/input-full.log"
    grep -qi 'No space left' "$fixture_dir/input-full.log"
    docker exec "$container" sh -eu -c '[ "$(cat /srv/rustsible/plain/download)" = original ]; [ -z "$(ls -A "$1")" ]' sh "$temporary"
    printf 'Input staging exhaustion passed (%s): structured failure, unchanged destination, empty temporary directory\n' "$temporary"
done
# Run the entire control/target pipeline inside the minimal environment too;
# host-side Python/curl availability cannot accidentally satisfy a dependency.
docker exec "$container" mkdir -p /opt/rustsible /srv/local-pipeline
docker cp "$controller_binary" "$container:/opt/rustsible/rustsible" >/dev/null
if [[ $libc != musl ]]; then docker cp "$worker_dir" "$container:/opt/rustsible/workers" >/dev/null; fi
docker exec "$container" chmod 0755 /opt/rustsible/rustsible
docker exec -i "$container" sh -c 'cat > /opt/rustsible/local.yml' <<'EOF'
- hosts: localhost
  tasks:
    - copy:
        src: /srv/http/large.bin
        dest: /srv/local-pipeline/binary
    - template:
        content: 'native {{ ansible_architecture }} {{ 1 + 1 }}'
        dest: /srv/local-pipeline/template
    - get_url:
        url: https://127.0.0.1:18081/hello
        dest: /srv/local-pipeline/https
    - get_url:
        url: http://must-not-resolve.invalid/hello
        dest: /srv/local-pipeline/proxy
    - lineinfile:
        path: /srv/local-pipeline/config
        line: enabled=true
        create: true
        mode: '0600'
EOF
docker exec "$container" sh -eu -c '
    ! command -v python3; ! command -v python; ! command -v curl
    printf "localhost ansible_connection=local\n" > /opt/rustsible/inventory
    /opt/rustsible/rustsible playbook /opt/rustsible/local.yml -i /opt/rustsible/inventory --check
    [ -z "$(find /srv/local-pipeline -mindepth 1)" ]
    /opt/rustsible/rustsible playbook /opt/rustsible/local.yml -i /opt/rustsible/inventory
    /opt/rustsible/rustsible playbook /opt/rustsible/local.yml -i /opt/rustsible/inventory > /opt/rustsible/local-second.log
    grep -q "changed=0 unreachable=0 failed=0" /opt/rustsible/local-second.log
    cmp /srv/http/large.bin /srv/local-pipeline/binary
    [ "$(cat /srv/local-pipeline/template)" = "native $(uname -m) 2" ]
    [ "$(cat /srv/local-pipeline/https)" = hello ]
    [ "$(cat /srv/local-pipeline/proxy)" = hello ]
    [ "$(stat -c %a /srv/local-pipeline/config)" = 600 ]
    [ -z "$(find /tmp /srv/local-pipeline -type d -name ".rustsible-*")" ]
' > "$fixture_dir/control-and-target-no-python.log"
printf 'Control and target pipeline passed inside %s without Python/curl: facts, stream, template, HTTPS, proxy, lineinfile, check and idempotency\n' "$libc"
