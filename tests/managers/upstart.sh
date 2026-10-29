#!/usr/bin/env bash
set -euo pipefail
controller=$(realpath -- "$1")
workers=$(realpath -- "$2")
http=$(realpath -- "$3")
fixture=$(mktemp -d)
container=''
cleanup() {
    local result=$?
    if (( result != 0 )); then
        for log in "$fixture"/*.log; do [[ ! -f $log ]] || cat "$log" >&2; done
        [[ -z $container ]] || docker logs --tail 25 "$container" >&2
    fi
    if [[ -n $container ]] && ! docker rm -f "$container" >/dev/null; then result=1; fi
    rm -rf -- "$fixture"
    exit "$result"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
# Ubuntu's image diverts initctl to /bin/true for package installation. Restore
# its real client before booting Upstart as PID 1 in a private container.
container=$(docker create --rm --network none --privileged --cgroupns=private --tmpfs /run ubuntu:14.04 sh -c 'cp /sbin/initctl.distrib /sbin/initctl; exec /sbin/init')
docker start "$container" >/dev/null
docker exec "$container" mkdir -p /opt/rustsible/workers /srv/http
docker cp "$controller" "$container:/opt/rustsible/rustsible" >/dev/null
docker cp "$workers/." "$container:/opt/rustsible/workers/" >/dev/null
docker cp "$http" "$container:/opt/rustsible/http_fixture" >/dev/null
docker exec "$container" sh -eu -c '
    chmod +x /opt/rustsible/rustsible /opt/rustsible/http_fixture /opt/rustsible/workers/rustsible-worker-*
    printf "localhost ansible_connection=local\n" > /opt/rustsible/inventory
    printf hello > /srv/http/hello
    printf "description \"Rustsible fixture\"\nexec /opt/rustsible/http_fixture 127.0.0.1:18090 /srv/http\n" > /etc/init/rustsible-fixture.conf
    initctl reload-configuration
    [ "$(readlink /proc/1/exe)" = /sbin/init ]
    initctl status rustsible-fixture | grep -q stop/waiting
'
run() { docker exec "$container" /opt/rustsible/rustsible --worker-dir /opt/rustsible/workers playbook /opt/rustsible/task.yml -i /opt/rustsible/inventory "$@"; }
play() {
    cat > "$fixture/task.yml" <<EOF
- hosts: all
  gather_facts: false
  tasks:
    - service:
        name: rustsible-fixture
        state: $1
EOF
    docker cp "$fixture/task.yml" "$container:/opt/rustsible/task.yml" >/dev/null
}
play started
run --check > "$fixture/check.log"
docker exec "$container" sh -eu -c 'initctl status rustsible-fixture | grep -q stop/waiting; [ ! -e /srv/http/health ]'
run > "$fixture/start.log"
run > "$fixture/noop.log"
grep -q 'changed=0 unreachable=0 failed=0' "$fixture/noop.log"
cat >> "$fixture/task.yml" <<'EOF'
    - command: /bin/true
    - get_url:
        url: http://127.0.0.1:18090/hello
        dest: /srv/http/health
EOF
docker cp "$fixture/task.yml" "$container:/opt/rustsible/task.yml" >/dev/null
run > "$fixture/health.log"
docker exec "$container" sh -eu -c 'initctl status rustsible-fixture | grep -q start/running; [ "$(cat /srv/http/health)" = hello ]'
play stopped
run --check > "$fixture/stop-check.log"
docker exec "$container" sh -eu -c 'initctl status rustsible-fixture | grep -q start/running'
run > "$fixture/stop.log"
run > "$fixture/stop-noop.log"
grep -q 'changed=0 unreachable=0 failed=0' "$fixture/stop-noop.log"
docker exec "$container" sh -eu -c '
    initctl status rustsible-fixture | grep -q stop/waiting
    for path in /tmp/.rustsible-*; do [ ! -e "$path" ]; done
'
printf 'Native Upstart acceptance passed: real PID 1, initctl, HTTP daemon lifetime, later synchronous task, idempotency, check and stop\n'
