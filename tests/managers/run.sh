#!/usr/bin/env bash
set -euo pipefail
repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
controller=$(realpath -- "$1")
workers=$(realpath -- "$2")
manager=${3:?expected dnf, yum, zypper or pacman}
fixture=$(mktemp -d)
container=''
cleanup() {
    local result=$?
    if (( result != 0 )); then
        for log in "$fixture"/*.log; do [[ ! -f $log ]] || cat "$log" >&2; done
    fi
    if [[ -n $container ]] && ! docker rm -f "$container" >/dev/null; then result=1; fi
    rm -rf -- "$fixture"
    exit "$result"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
build_args=()
if [[ -n ${CODEX_PROXY_CERT:-} ]]; then build_args+=(--secret "id=proxy_ca,src=$CODEX_PROXY_CERT"); fi
case "$manager" in
    dnf|yum|zypper)
        docker build -q -f "$repo_dir/tests/managers/Dockerfile.rpm" "${build_args[@]}" -t rustsible-manager-dnf:local "$repo_dir/tests/managers" >/dev/null
        if [[ $manager != dnf ]]; then
            base=centos:7
            [[ $manager != zypper ]] || base=opensuse/leap:16.0
            docker build -q -f "$repo_dir/tests/managers/Dockerfile.other-rpm" --build-arg "BASE=$base" -t "rustsible-manager-$manager:local" "$repo_dir/tests/managers" >/dev/null
        fi
        ;;
    pacman) docker build -q -f "$repo_dir/tests/managers/Dockerfile.arch" -t rustsible-manager-pacman:local "$repo_dir/tests/managers" >/dev/null;;
    *) echo 'Unknown package fixture' >&2; exit 1;;
esac
container=$(docker run -d --rm --network none "rustsible-manager-$manager:local")
docker exec "$container" mkdir -p /opt/rustsible/workers
docker cp "$controller" "$container:/opt/rustsible/rustsible" >/dev/null
docker cp "$workers/." "$container:/opt/rustsible/workers/" >/dev/null
docker exec "$container" sh -eu -c '
    chmod +x /opt/rustsible/rustsible /opt/rustsible/workers/rustsible-worker-*
    printf "localhost ansible_connection=local\n" > /opt/rustsible/inventory
    cp -a /srv/versions/1.0 /srv/repo
'
case "$manager" in
    dnf|yum)
        docker exec "$container" sh -eu -c '
            rm -f /etc/yum.repos.d/*.repo
            printf "[fixture]\nname=fixture\nbaseurl=file:///srv/repo\nenabled=1\ngpgcheck=0\n" > /etc/yum.repos.d/fixture.repo
        ';;
    zypper)
        docker exec "$container" sh -eu -c '
            rm -f /etc/zypp/services.d/*.service
            rm -f /etc/zypp/repos.d/*.repo
            printf "[fixture]\nname=fixture\nbaseurl=file:///srv/repo\nenabled=1\ngpgcheck=0\nautorefresh=0\n" > /etc/zypp/repos.d/fixture.repo
        ';;
    pacman)
        docker exec "$container" sh -eu -c '
            printf "[options]\nArchitecture = auto\nSigLevel = Never\n[fixture]\nServer = file:///srv/repo\n" > /etc/pacman.conf
        ';;
esac
run() {
    docker exec "$container" /opt/rustsible/rustsible --worker-dir /opt/rustsible/workers playbook /opt/rustsible/task.yml -i /opt/rustsible/inventory "$@"
}
play() {
    local state=$1 update=$2
    cat > "$fixture/task.yml" <<EOF
- hosts: all
  gather_facts: false
  tasks:
    - package:
        name: [rustsible-fixture-one, rustsible-fixture-two]
        state: $state
        update_cache: $update
EOF
    docker cp "$fixture/task.yml" "$container:/opt/rustsible/task.yml" >/dev/null
}
snapshot() {
    docker exec "$container" sh -eu -c '
        walk() {
            for path in "$1"/* "$1"/.[!.]*; do
                [ -e "$path" ] || continue
                # Berkeley DB coordination regions change even for rpm -q;
                # persistent package indexes and payload/cache bytes must not.
                case "$path" in */__db.*) continue;; esac
                if [ -d "$path" ]; then walk "$path"; elif [ -f "$path" ]; then sha256sum "$path"; fi
            done
        }
        for dir in /var/cache/dnf /var/cache/yum /var/cache/zypp /var/lib/pacman /var/lib/rpm /usr/lib/sysimage/rpm; do
            [ ! -d "$dir" ] || walk "$dir"
        done
    ' | sort
}
versions() {
    docker exec "$container" sh -eu -c '
        for name in rustsible-fixture-one rustsible-fixture-two; do
            [ "$(cat "/usr/share/$name/version")" = "$1" ]
            if command -v rpm >/dev/null; then rpm -q "$name"; else pacman -Q "$name"; fi
        done
    ' sh "$1"
}
play present true
snapshot > "$fixture/before"
run --check > "$fixture/check.log"
snapshot > "$fixture/after"
diff -u "$fixture/before" "$fixture/after"
run > "$fixture/install.log"
versions 1.0-1
play present false
run > "$fixture/noop.log"
grep -q 'changed=0 unreachable=0 failed=0' "$fixture/noop.log"
if [[ $manager != zypper ]]; then
    docker exec "$container" sh -eu -c 'rm -rf /srv/repo; cp -a /srv/versions/2.0 /srv/repo'
    play latest true
    run > "$fixture/upgrade.log"
    versions 2.0-1
    play latest false
    snapshot > "$fixture/latest-before"
    run --check > "$fixture/latest-check.log"
    snapshot > "$fixture/latest-after"
    diff -u "$fixture/latest-before" "$fixture/latest-after"
    run > "$fixture/latest-noop.log"
    grep -q 'changed=0 unreachable=0 failed=0' "$fixture/latest-noop.log"
else
    play latest true
    snapshot > "$fixture/latest-before"
    if run > "$fixture/unsupported.log"; then echo 'Unsupported zypper latest accepted' >&2; exit 1; fi
    snapshot > "$fixture/latest-after"
    diff -u "$fixture/latest-before" "$fixture/latest-after"
fi
play absent false
run > "$fixture/remove.log"
run > "$fixture/remove-noop.log"
grep -q 'changed=0 unreachable=0 failed=0' "$fixture/remove-noop.log"
docker exec "$container" sh -eu -c '
    [ ! -e /usr/share/rustsible-fixture-one/version ]
    [ ! -e /usr/share/rustsible-fixture-two/version ]
    for path in /tmp/.rustsible-*; do [ ! -e "$path" ]; done
'
printf 'Native package acceptance passed: %s, real two-package transaction, local repository, offline, version files/database, check invariance, idempotency and removal\n' "$manager"
