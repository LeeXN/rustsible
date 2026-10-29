# The collection was assembled from static ELF artifacts before acceptance.
# Packing and installation inspect bytes; they do not execute package content.
collection_source=$(realpath -- "$5")
"$controller_binary" collection pack "$collection_source" --output "$fixture_dir/native.tar.gz"
"$controller_binary" collection verify "$fixture_dir/native.tar.gz" > "$fixture_dir/native-verify.log"
"$controller_binary" collection install "$fixture_dir/native.tar.gz" --path "$fixture_dir/installed"
collection_paths=$fixture_dir/installed
cat > "$fixture_dir/record.json" <<'JSON'
{"title":"Native Provider","literal":"{{ missing_variable.must_not_execute }}","count":7}
JSON
cat > "$fixture_dir/extensions.yml" <<EOF
- hosts: all
  gather_facts: false
  collections: [example.native]
  vars:
    ansible_host: 127.0.0.1
    expected_arch: $target_arch
    expected_uids:
      plain: $(docker exec "$container" id -u deploy)
      root: 0
      app: $(docker exec "$container" id -u app)
    record: "{{ lookup('example.native.record', '$fixture_dir/record.json') }}"
  roles: [example.native.demo]
  tasks:
    - assert:
        that:
          - native_probe.probe.arch == expected_arch
          - native_probe.probe.uid == expected_uids[inventory_hostname]
          - record.count == 7
          - record.title | example.native.slug == 'native-provider'
          - record.literal == '{{ missing_variable.must_not_execute }}'
    - copy:
        content: '{{ record.title | example.native.slug }}:{{ native_probe.probe.uid }}:{{ native_probe.probe.arch }}'
        dest: /srv/rustsible/{{ inventory_hostname }}/native-result
        mode: '0600'
EOF
# Compare literal delimiters without introducing a template in the assertion.
sed -i "/record.literal ==/d" "$fixture_dir/extensions.yml"
extensions_snapshot() {
    docker exec "$container" sh -eu -c 'find /srv/rustsible -type f -exec sha256sum {} \;; find /srv/rustsible -exec stat -c "%n:%a:%u:%g" {} \;'
}
extensions_snapshot > "$fixture_dir/extensions-before"
rustsible playbook "$fixture_dir/extensions.yml" --collections-path "$collection_paths" -i "$fixture_dir/inventory" --check --diff > "$fixture_dir/extensions-check.log"
extensions_snapshot > "$fixture_dir/extensions-after"
diff -u "$fixture_dir/extensions-before" "$fixture_dir/extensions-after"
rustsible playbook "$fixture_dir/extensions.yml" --collections-path "$collection_paths" -i "$fixture_dir/inventory" > "$fixture_dir/extensions-first.log"
rustsible playbook "$fixture_dir/extensions.yml" --collections-path "$collection_paths" -i "$fixture_dir/inventory" > "$fixture_dir/extensions-second.log"
if grep -Eq 'changed=[1-9]' "$fixture_dir/extensions-second.log"; then echo 'Native collection deployment was not idempotent' >&2; exit 1; fi
cat > "$fixture_dir/extensions-async.yml" <<EOF
- hosts: all
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - example.native.probe:
        path: /etc/passwd
        delay_ms: 500
      async: 30
      poll: 0
      register: job
    - copy:
        content: '{{ job.ansible_job_id }}'
        dest: $fixture_dir/native-job-{{ inventory_hostname }}
      delegate_to: localhost
EOF
rustsible playbook "$fixture_dir/extensions-async.yml" --collections-path "$collection_paths" -i "$fixture_dir/inventory" > "$fixture_dir/extensions-async.log"
printf 'jobs:\n' > "$fixture_dir/extensions-jobs.yml"
for name in plain root app; do
    printf '  %s: "%s"\n' "$name" "$(cat "$fixture_dir/native-job-$name")" >> "$fixture_dir/extensions-jobs.yml"
done
cat > "$fixture_dir/extensions-query.yml" <<EOF
- hosts: all
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  vars_files: [$fixture_dir/extensions-jobs.yml]
  tasks:
    - async_status:
        jid: '{{ jobs[inventory_hostname] }}'
      register: completed
      until: completed is finished
      retries: 30
      delay: 0.1
    - assert:
        that: completed.probe.exists
    - async_status:
        jid: '{{ jobs[inventory_hostname] }}'
        mode: cleanup
EOF
# A new controller invocation has no collection configured: the retained worker
# and program must still finish independently and expose the standard result.
rustsible playbook "$fixture_dir/extensions-query.yml" -i "$fixture_dir/inventory" > "$fixture_dir/extensions-query.log"
for pair in plain:deploy root:root app:app; do
    name=${pair%:*}; user=${pair#*:}
    docker exec "$container" sh -eu -c '
        [ "$(cat "/srv/rustsible/$1/native-result")" = "native-provider:$(id -u "$2"):$3" ]
        [ "$(stat -c %U:%a "/srv/rustsible/$1/native-result")" = "$2:600" ]
        [ -z "$(find /tmp -type d -name ".rustsible-worker-*")" ]
        ! command -v python3
        ! command -v curl
    ' sh "$name" "$user" "$target_arch"
done
# The controller itself also runs offline in the Python/curl-free target.
docker cp "$controller_binary" "$container:/usr/local/bin/rustsible-native-acceptance" >/dev/null
docker cp "$worker_dir" "$container:/tmp/native-workers" >/dev/null
docker cp "$fixture_dir/native.tar.gz" "$container:/tmp/native.tar.gz" >/dev/null
docker exec "$container" sh -eu -c '
    chmod +x /usr/local/bin/rustsible-native-acceptance
    rustsible-native-acceptance collection verify /tmp/native.tar.gz >/dev/null
    rustsible-native-acceptance collection install /tmp/native.tar.gz --path /tmp/native-collections
    printf "localhost ansible_connection=local\n" > /tmp/native-inventory
    rustsible-native-acceptance --worker-dir /tmp/native-workers ad-hoc localhost -i /tmp/native-inventory --collections-path /tmp/native-collections -m example.native.probe -a "path=/etc/passwd"
    [ -z "$(find /tmp -type d -name ".rustsible-worker-*")" ]
'
printf 'Native collections, providers, identities, async and offline controller passed (%s/%s)\n' "$target_arch" "$libc"
