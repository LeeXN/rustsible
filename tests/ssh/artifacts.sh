docker exec "$container" sh -eu -c '
    mkdir -p /srv/artifact-source /srv/archive-content/nested
    printf version-one > /srv/artifact-source/version
    git -C /srv/artifact-source init -q
    git -C /srv/artifact-source -c user.name=Fixture -c user.email=fixture@example.invalid add .
    git -C /srv/artifact-source -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm first
    git -C /srv/artifact-source tag v1
    git config --system --add safe.directory /srv/artifact-source
    git config --system --add safe.directory /srv/artifact-source/.git
    printf executable-data > /srv/archive-content/nested/tool
    chmod 0755 /srv/archive-content/nested/tool
    tar -cf /srv/artifact.tar -C /srv/archive-content nested
    tar -czf /srv/artifact.tar.gz -C /srv/archive-content nested
'
docker cp "$container:/srv/artifact.tar" "$fixture_dir/source.tar" >/dev/null
cat > "$fixture_dir/artifacts.yml" <<'YAML'
- hosts: all
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - git:
        repo: /srv/artifact-source
        dest: /srv/rustsible/{{ inventory_hostname }}/checkout
        version: v1
      register: checkout
    - file:
        path: /srv/rustsible/{{ inventory_hostname }}/files
        state: directory
        mode: '0755'
    - unarchive:
        src: source.tar
        dest: /srv/rustsible/{{ inventory_hostname }}/files
      register: unpacked
    - unarchive:
        src: /srv/artifact.tar.gz
        dest: /srv/rustsible/{{ inventory_hostname }}/files
        remote_src: true
      register: current
YAML
snapshot_artifacts() {
    docker exec "$container" sh -eu -c '
        find /srv/rustsible -type f -exec sha256sum {} \;
        find /srv/rustsible -exec stat -c "%n:%a:%u:%g" {} \;
    '
}
# Existing destination directories make archive check exercise its read-only
# prediction rather than fail on a directory another check-only task predicts.
for name in plain root app; do
    user=$name; [[ $name != plain ]] || user=deploy
    docker exec "$container" install -d -o "$user" -g "$user" -m 0755 "/srv/rustsible/$name/files"
done
snapshot_artifacts > "$fixture_dir/artifacts-before"
rustsible playbook "$fixture_dir/artifacts.yml" -i "$fixture_dir/inventory" --check > "$fixture_dir/artifacts-check.log"
snapshot_artifacts > "$fixture_dir/artifacts-after"
diff -u "$fixture_dir/artifacts-before" "$fixture_dir/artifacts-after"
rustsible playbook "$fixture_dir/artifacts.yml" -i "$fixture_dir/inventory" > "$fixture_dir/artifacts-first.log"
rustsible playbook "$fixture_dir/artifacts.yml" -i "$fixture_dir/inventory" > "$fixture_dir/artifacts-second.log"
if grep -Eq 'changed=[1-9]' "$fixture_dir/artifacts-second.log"; then echo 'Artifact deployment was not idempotent' >&2; exit 1; fi
for pair in plain:deploy root:root app:app; do
    name=${pair%:*}; user=${pair#*:}
    docker exec "$container" sh -eu -c '
        [ "$(cat "/srv/rustsible/$1/checkout/version")" = version-one ]
        [ "$(cat "/srv/rustsible/$1/files/nested/tool")" = executable-data ]
        [ "$(stat -c %a "/srv/rustsible/$1/files/nested/tool")" = 755 ]
        [ "$(stat -c %U "/srv/rustsible/$1/files/nested/tool")" = "$2" ]
        [ "$(stat -c %U "/srv/rustsible/$1/checkout/version")" = "$2" ]
        [ -z "$(find /tmp -type d -name ".rustsible-worker-*")" ]
    ' sh "$name" "$user"
done
docker exec "$container" sh -eu -c '
    mkdir -p /srv/archive-failure /small/archive
    dd if=/dev/zero of=/srv/archive-failure/kept bs=1024 count=256 2>/dev/null
    tar -cf /srv/failure.tar -C /srv/archive-failure kept
    printf retained > /small/archive/kept
    chmod 0755 /small/archive/kept
    stat -c "%i:%a:%u:%g" /small/archive/kept > /srv/kept-before
'
if rustsible ad-hoc root -i "$fixture_dir/inventory" -e ansible_host=127.0.0.1 -m unarchive -a '{"src":"/srv/failure.tar","dest":"/small/archive","remote_src":true}' > "$fixture_dir/artifacts-full.log" 2>&1; then
    echo 'Unarchive unexpectedly succeeded on a full destination filesystem' >&2; exit 1
fi
docker exec "$container" sh -eu -c '
    [ "$(cat /small/archive/kept)" = retained ]
    [ "$(stat -c "%i:%a:%u:%g" /small/archive/kept)" = "$(cat /srv/kept-before)" ]
    [ -z "$(find /small -type d -name ".rustsible-write-*")" ]
'
printf 'SSH git/unarchive passed (%s/%s): three identities, pinned revision, controller tar/remote gzip, executable ownership, idempotency, check and cleanup\n' "$target_arch" "$libc"
