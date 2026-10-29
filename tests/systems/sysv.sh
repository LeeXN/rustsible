docker exec "$container" sh -eu -c '[ ! -d /run/systemd/system ]; command -v service; command -v start-stop-daemon'
docker exec "$container" mkdir -p /srv/http
docker cp "$http_fixture" "$container:/srv/http/server" >/dev/null
docker exec "$container" chmod 0755 /srv/http/server
cat > "$fixture_dir/init-script" <<'EOF'
#!/bin/sh
case "$1" in
start) start-stop-daemon --start --background --make-pidfile --pidfile /run/rustsible-sysv.pid --startas /srv/http/server -- 127.0.0.1:18090 /srv/http;;
status) start-stop-daemon --status --pidfile /run/rustsible-sysv.pid;;
stop) start-stop-daemon --stop --retry TERM/5/KILL/5 --pidfile /run/rustsible-sysv.pid; rm -f /run/rustsible-sysv.pid;;
*) exit 4;;
esac
EOF
docker cp "$fixture_dir/init-script" "$container:/etc/init.d/rustsible-sysv" >/dev/null
docker exec "$container" chmod 0755 /etc/init.d/rustsible-sysv
cat > "$fixture_dir/sysv.yml" <<'EOF'
- hosts: root
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - service:
        name: rustsible-sysv
        state: started
    - command: /bin/true
    - get_url:
        url: http://127.0.0.1:18090/hello
        dest: /srv/rustsible/root/health
EOF
# A daemon can take a moment to bind after start-stop-daemon --background.
# Start it through the module first; the repeated deployment below validates
# idempotency and that a later command did not kill the adopted daemon.
sed '/    - command:/,$d' "$fixture_dir/sysv.yml" > "$fixture_dir/sysv-start.yml"
rustsible playbook "$fixture_dir/sysv-start.yml" -i "$fixture_dir/inventory" > "$fixture_dir/sysv-start.log"
for _ in {1..100}; do
    if docker exec "$container" service rustsible-sysv status >/dev/null 2>&1; then break; fi
    sleep 0.05
done
rustsible playbook "$fixture_dir/sysv.yml" -i "$fixture_dir/inventory" > "$fixture_dir/sysv-first.log"
rustsible playbook "$fixture_dir/sysv.yml" -i "$fixture_dir/inventory" > "$fixture_dir/sysv-second.log"
# /bin/true is the only always-changed task.
grep -q 'changed=1 unreachable=0 failed=0' "$fixture_dir/sysv-second.log"
docker exec "$container" sh -eu -c '
    service rustsible-sysv status
    [ "$(cat /srv/rustsible/root/health)" = hello ]
'
sed 's/state: started/state: stopped/' "$fixture_dir/sysv-start.yml" > "$fixture_dir/sysv-stop.yml"
rustsible playbook "$fixture_dir/sysv-stop.yml" -i "$fixture_dir/inventory" > "$fixture_dir/sysv-stop.log"
docker exec "$container" sh -eu -c '[ ! -e /run/rustsible-sysv.pid ]; [ -z "$(find /tmp -type d -name ".rustsible-*")" ]'
printf 'Native SysV acceptance passed (%s): real init script, daemon ownership/lifetime, later synchronous task, HTTP health and stop\n' "$target_arch"
