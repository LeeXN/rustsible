# Disposable container with a real systemd PID 1, real passwd/shadow and dpkg.
docker exec "$container" sh -eu -c '
    [ "$(cat /proc/1/comm)" = systemd ]
    systemctl is-active ssh
    ! command -v python3; ! command -v curl
    groupadd -g 26000 rustsible-fixture
    groupadd -g 26001 rustsible-extra
    mkdir -p /srv/http/repo
'
docker cp "$http_fixture" "$container:/srv/http/server" >/dev/null
docker exec "$container" chmod 0755 /srv/http/server
docker exec -d "$container" sh -c 'exec /srv/http/server 127.0.0.1:18080 /srv/http >/srv/http/server.log 2>&1'
# The repo contains two independent packages; version replacement tests that
# candidate queries happen after refresh, not against the prior cached index.
make_repo() {
    docker exec "$container" sh -eu -c '
        version=$1
        for name in rustsible-fixture-one rustsible-fixture-two; do
            base=/srv/build/$name
            mkdir -p "$base/DEBIAN" "$base/usr/share/$name"
            printf "Package: %s\nVersion: %s\nArchitecture: all\nMaintainer: Rustsible test\nDescription: isolated native acceptance fixture\n" "$name" "$version" > "$base/DEBIAN/control"
            printf "%s" "$version" > "$base/usr/share/$name/version"
            dpkg-deb --build "$base" "/srv/http/repo/$name.deb" >/dev/null
        done
        : > /srv/http/repo/Packages
        for file in /srv/http/repo/*.deb; do
            dpkg-deb -f "$file" >> /srv/http/repo/Packages
            printf "Filename: %s\nSize: %s\nSHA256: %s\n\n" "$(basename "$file")" "$(stat -c %s "$file")" "$(sha256sum "$file" | cut -d " " -f1)" >> /srv/http/repo/Packages
        done
    ' sh "$1"
}
make_repo 1.0-1
docker exec "$container" sh -eu -c '
    for file in /etc/apt/sources.list /etc/apt/sources.list.d/*; do [ ! -f "$file" ] || rm "$file"; done
    printf "deb [trusted=yes] http://127.0.0.1:18080/repo ./\n" > /etc/apt/sources.list
'
cat > "$fixture_dir/service.unit" <<'EOF'
[Unit]
Description=Rustsible real systemd acceptance
[Service]
ExecStart=/srv/http/server 127.0.0.1:18090 /srv/http
[Install]
WantedBy=multi-user.target
EOF
cat > "$fixture_dir/systems.yml" <<EOF
- hosts: root
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - user:
        name: rustsible-fixture
        uid: 26000
        groups: [rustsible-extra]
        shell: /bin/sh
        comment: native account
        password: '!native-shadow-canary'
      no_log: true
    - package:
        name: [rustsible-fixture-one, rustsible-fixture-two]
        state: present
      check_mode: true
      register: package_plan
    - package:
        name: [rustsible-fixture-one, rustsible-fixture-two]
        state: present
        update_cache: true
      when: package_plan.changed
    - copy:
        src: $fixture_dir/service.unit
        dest: /etc/systemd/system/rustsible-fixture.service
      register: unit
    - systemd:
        daemon_reload: true
      when: unit.changed
    - systemd:
        name: rustsible-fixture.service
        enabled: true
        state: started
EOF
system_snapshot() {
    docker exec "$container" sh -eu -c '
        sha256sum /etc/passwd /etc/group /etc/shadow /var/lib/dpkg/status
        find /var/lib/apt/lists -type f -exec sha256sum {} \;
        systemctl is-active ssh
        if [ -e /etc/systemd/system/rustsible-fixture.service ]; then sha256sum /etc/systemd/system/rustsible-fixture.service; fi
    '
}
# The unit does not exist during dry-run; exercise account/package prediction
# separately so no claim is made about an uninstalled unit being startable.
sed '/    - copy:/,$d' "$fixture_dir/systems.yml" > "$fixture_dir/check.yml"
system_snapshot > "$fixture_dir/before"
rustsible playbook "$fixture_dir/check.yml" -i "$fixture_dir/inventory" --check > "$fixture_dir/system-check.log"
system_snapshot > "$fixture_dir/after"
diff -u "$fixture_dir/before" "$fixture_dir/after"
rustsible playbook "$fixture_dir/systems.yml" -i "$fixture_dir/inventory" -vv > "$fixture_dir/system-first.log" 2>&1
! grep -q 'native-shadow-canary' "$fixture_dir/system-first.log"
rustsible playbook "$fixture_dir/systems.yml" -i "$fixture_dir/inventory" > "$fixture_dir/system-second.log"
grep -q 'changed=0 unreachable=0 failed=0' "$fixture_dir/system-second.log"
docker exec "$container" sh -eu -c '
    [ "$(id -u rustsible-fixture)" = 26000 ]
    [ "$(id -g rustsible-fixture)" = 26000 ]
    id -nG rustsible-fixture | grep -q rustsible-extra
    [ "$(getent shadow rustsible-fixture | cut -d: -f2)" = "!native-shadow-canary" ]
    [ -d /home/rustsible-fixture ]
    [ "$(stat -c %u /home/rustsible-fixture)" = 26000 ]
    systemctl is-enabled rustsible-fixture.service
    systemctl is-active rustsible-fixture.service
    for package in rustsible-fixture-one rustsible-fixture-two; do
        [ "$(dpkg-query -W -f="\${Version}" "$package")" = 1.0-1 ]
        [ "$(cat "/usr/share/$package/version")" = 1.0-1 ]
    done
'
# Unprivileged identities may predict but cannot silently escalate a mutation.
cat > "$fixture_dir/permission.yml" <<'EOF'
- hosts: plain:app
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - user:
        name: rustsible-must-not-create
        create_home: false
EOF
rustsible playbook "$fixture_dir/permission.yml" -i "$fixture_dir/inventory" --check > "$fixture_dir/permission-check.log"
if rustsible playbook "$fixture_dir/permission.yml" -i "$fixture_dir/inventory" > "$fixture_dir/permission.log" 2>&1; then
    echo 'Unprivileged account mutation unexpectedly succeeded' >&2; exit 1
fi
docker exec "$container" sh -c '! getent passwd rustsible-must-not-create'
# A missing existing home is a supported create_home action, not just a flag
# passed to useradd for brand-new accounts.
docker exec "$container" rm -r /home/rustsible-fixture
rustsible playbook "$fixture_dir/systems.yml" -i "$fixture_dir/inventory" > "$fixture_dir/home-repair.log"
docker exec "$container" sh -eu -c '[ -d /home/rustsible-fixture ]; [ "$(stat -c %u /home/rustsible-fixture)" = 26000 ]'
# The same modules make the same no-op decisions through local transport and
# when executed as root, a normal user or non-root become.
docker exec "$container" mkdir -p /opt/rustsible
docker cp "$controller_binary" "$container:/opt/rustsible/rustsible" >/dev/null
docker cp "$worker_dir" "$container:/opt/rustsible/workers" >/dev/null
docker exec "$container" chmod 0755 /opt/rustsible/rustsible
docker exec -i "$container" sh -c 'cat > /opt/rustsible/noop.yml' <<'EOF'
- hosts: localhost
  tasks:
    - user:
        name: rustsible-fixture
        create_home: false
    - package:
        name: [rustsible-fixture-one, rustsible-fixture-two]
        state: present
    - systemd:
        name: rustsible-fixture.service
        state: started
        enabled: true
EOF
docker exec "$container" sh -c 'printf "localhost ansible_connection=local\n" > /opt/rustsible/inventory'
for user in root deploy app; do
    docker exec -u "$user" "$container" /opt/rustsible/rustsible playbook /opt/rustsible/noop.yml -i /opt/rustsible/inventory > "$fixture_dir/local-$user.log"
    grep -q 'changed=0 unreachable=0 failed=0' "$fixture_dir/local-$user.log"
done
# Query the service through the native HTTP module to prove it is doing work.
cat > "$fixture_dir/health.yml" <<'EOF'
- hosts: root
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - get_url:
        url: http://127.0.0.1:18090/hello
        dest: /srv/rustsible/root/health
        force: true
EOF
rustsible playbook "$fixture_dir/health.yml" -i "$fixture_dir/inventory" > "$fixture_dir/system-health.log"
docker exec "$container" test "$(docker exec "$container" cat /srv/rustsible/root/health)" = hello
make_repo 2.0-1
cat > "$fixture_dir/upgrade.yml" <<'EOF'
- hosts: root
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - package:
        name: [rustsible-fixture-one, rustsible-fixture-two]
        state: latest
        update_cache: true
EOF
rustsible playbook "$fixture_dir/upgrade.yml" -i "$fixture_dir/inventory" > "$fixture_dir/system-upgrade.log"
docker exec "$container" sh -eu -c '
    for package in rustsible-fixture-one rustsible-fixture-two; do
        [ "$(cat "/usr/share/$package/version")" = 2.0-1 ]
    done
'
cat > "$fixture_dir/remove.yml" <<'EOF'
- hosts: root
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - systemd:
        name: rustsible-fixture.service
        state: stopped
        enabled: false
    - package:
        name: [rustsible-fixture-one, rustsible-fixture-two]
        state: absent
    - user:
        name: rustsible-fixture
        state: absent
        remove: true
EOF
rustsible playbook "$fixture_dir/remove.yml" -i "$fixture_dir/inventory" > "$fixture_dir/system-remove.log"
docker exec "$container" sh -eu -c '
    ! systemctl is-active rustsible-fixture.service
    ! systemctl is-enabled rustsible-fixture.service
    ! getent passwd rustsible-fixture
    [ ! -e /home/rustsible-fixture ]
    [ ! -e /usr/share/rustsible-fixture-one/version ]
    [ ! -e /usr/share/rustsible-fixture-two/version ]
    [ -z "$(find /tmp /srv -type d -name ".rustsible-*")" ]
'
printf 'Native system acceptance passed (%s): real systemd PID 1 and HTTP health, account/shadow/home/groups, batch APT install/upgrade/removal, cache-before-candidate, check invariance and idempotency\n' "$target_arch"
