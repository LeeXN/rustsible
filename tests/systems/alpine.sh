# Real OpenRC and APK tools in a Python/curl-free musl environment. The repo
# index and package retain Alpine signatures; runtime never uses the Internet.
docker exec "$container" sh -eu -c '
    ! command -v python3; ! command -v curl
    mkdir -p /srv/http /run/openrc
    printf default > /run/openrc/softlevel
    printf "http://127.0.0.1:18080/repo\n" > /etc/apk/repositories
    groupadd -g 26000 rustsible-fixture
    architecture=$(apk --print-arch)
    mkdir -p "/srv/http/repo/$architecture"
    for file in /srv/http/repo/*.apk /srv/http/repo/*.tar.gz; do
        mv "$file" "/srv/http/repo/$architecture/"
    done
'
docker cp "$http_fixture" "$container:/srv/http/server" >/dev/null
docker exec "$container" chmod 0755 /srv/http/server
docker exec -d "$container" sh -c 'exec /srv/http/server 127.0.0.1:18080 /srv/http >/srv/http/http.log 2>&1'
cat > "$fixture_dir/openrc-service" <<'EOF'
#!/sbin/openrc-run
description="Rustsible real OpenRC fixture"
command="/srv/http/server"
command_args="127.0.0.1:18090 /srv/http"
command_background="yes"
pidfile="/run/rustsible-fixture.pid"
EOF
docker cp "$fixture_dir/openrc-service" "$container:/etc/init.d/rustsible-fixture" >/dev/null
docker exec "$container" chmod 0755 /etc/init.d/rustsible-fixture
cat > "$fixture_dir/alpine-systems.yml" <<'EOF'
- hosts: root
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - user:
        name: rustsible-fixture
        uid: 26000
        shell: /bin/sh
        create_home: true
    - package:
        name: tree
        state: present
      check_mode: true
      register: package_plan
    - package:
        name: tree
        update_cache: true
      when: package_plan.changed
    - service:
        name: rustsible-fixture
        state: started
        enabled: true
    - get_url:
        url: http://127.0.0.1:18090/hello
        dest: /srv/rustsible/root/health
        force: true
EOF
# The HTTP health target is not running before the first real start. Exercise
# dry-run only over user/package setup, and compare actual database bytes.
sed '/    - service:/,$d' "$fixture_dir/alpine-systems.yml" > "$fixture_dir/alpine-check.yml"
docker exec "$container" sh -c 'sha256sum /etc/passwd /etc/group /etc/shadow /lib/apk/db/installed' > "$fixture_dir/alpine-before"
rustsible playbook "$fixture_dir/alpine-check.yml" -i "$fixture_dir/inventory" --check > "$fixture_dir/alpine-check.log"
docker exec "$container" sh -c 'sha256sum /etc/passwd /etc/group /etc/shadow /lib/apk/db/installed' > "$fixture_dir/alpine-after"
diff -u "$fixture_dir/alpine-before" "$fixture_dir/alpine-after"
rustsible playbook "$fixture_dir/alpine-systems.yml" -i "$fixture_dir/inventory" > "$fixture_dir/alpine-first.log"
rustsible playbook "$fixture_dir/alpine-systems.yml" -i "$fixture_dir/inventory" > "$fixture_dir/alpine-second.log"
grep -q 'changed=0 unreachable=0 failed=0' "$fixture_dir/alpine-second.log"
docker exec "$container" sh -eu -c '
    [ "$(id -u rustsible-fixture)" = 26000 ]
    [ -d /home/rustsible-fixture ]
    apk info -e tree
    tree --version
    rc-service rustsible-fixture status
    rc-update show default | grep -q rustsible-fixture
    [ "$(cat /srv/rustsible/root/health)" = hello ]
'
cat > "$fixture_dir/alpine-remove.yml" <<'EOF'
- hosts: root
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - service:
        name: rustsible-fixture
        state: stopped
        enabled: false
    - package:
        name: tree
        state: absent
    - user:
        name: rustsible-fixture
        state: absent
        remove: true
EOF
rustsible playbook "$fixture_dir/alpine-remove.yml" -i "$fixture_dir/inventory" > "$fixture_dir/alpine-remove.log"
docker exec "$container" sh -eu -c '
    ! apk info -e tree
    ! getent passwd rustsible-fixture
    ! rc-service rustsible-fixture status
    [ ! -d /home/rustsible-fixture ]
    [ -z "$(find /tmp -type d -name ".rustsible-*")" ]
'
printf 'Native musl system acceptance passed (%s): real OpenRC daemon/HTTP health, signed local APK repo, account and home, install/removal, check invariance and idempotency\n' "$target_arch"
