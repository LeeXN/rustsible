# CI supplies extracted release binaries. All servers use the native runner
# architecture; controller emulation is a separate, explicitly named CI case.
source "$repo_dir/tests/ssh/rollout-setup.sh"
collection_source=$(realpath -- "$6")
"$controller_binary" collection pack "$collection_source" --output "$fixture_dir/native.tar.gz"
"$controller_binary" collection install "$fixture_dir/native.tar.gz" --path "$fixture_dir/collections"
mkdir -p "$fixture_dir/group_vars" "$fixture_dir/roles/deployment/tasks" "$fixture_dir/roles/deployment/templates"
printf 'release-vault-password\n' > "$fixture_dir/vault-password"
chmod 0600 "$fixture_dir/vault-password"
printf 'deployment_token: release-secret-canary\n' > "$fixture_dir/group_vars/rollout.yml"
"$controller_binary" vault encrypt "$fixture_dir/group_vars/rollout.yml" --vault-id "release@$fixture_dir/vault-password"
printf 'verified-offline-artifact\n' > "$fixture_dir/payload"
payload_sha=$(sha256sum "$fixture_dir/payload"); payload_sha=${payload_sha%% *}
for server in "${rollout_containers[@]}"; do
    docker cp "$http_fixture" "$server:/usr/local/bin/release-http" >/dev/null
    docker cp "$fixture_dir/payload" "$server:/srv/rustsible/rollout/payload-source" >/dev/null
    docker exec "$server" sh -eu -c '
        chmod +x /usr/local/bin/release-http
        /usr/local/bin/release-http 127.0.0.1:18080 /srv/rustsible/rollout >/tmp/release-http.log 2>&1 &
    '
done
cat > "$fixture_dir/roles/deployment/templates/config.j2" <<'EOF_TEMPLATE'
{{ deployment_version }}|{{ deployment_token }}|{{ inventory_hostname }}
EOF_TEMPLATE
cat > "$fixture_dir/roles/deployment/tasks/main.yml" <<EOF
- get_url:
    url: http://127.0.0.1:18080/payload-source
    dest: /srv/rustsible/rollout/artifact
    checksum: sha256:$payload_sha
    force: false
    mode: '0755'
- template:
    src: config.j2
    dest: /srv/rustsible/rollout/config
  no_log: true
  notify: reload deployed configuration
EOF
cat > "$fixture_dir/release.yml" <<'YAML'
- hosts: rollout
  gather_facts: true
  serial: [1, 2]
  throttle: 2
  vars:
    deployment_version: release-v1
    failure_host: ''
  pre_tasks:
    - assert:
        that: inventory_hostname == 'node1' or hostvars.node1.batch_done | default(false)
    - command:
        argv: [cat, /srv/rustsible/rollout/config]
      register: previous_configuration
      changed_when: false
      no_log: true
  tasks:
    - import_role:
        name: example.native.demo
    - block:
        - import_role:
            name: deployment
        - meta: flush_handlers
        - wait_for:
            path: /srv/rustsible/rollout/applied
            timeout: 10
          when: not ansible_check_mode
        - uri:
            url: http://127.0.0.1:18080/applied
            status_code: [200, 404]
            return_content: true
          register: health
          until: health.status == 200
          retries: 10
          delay: 0.1
          failed_when: health.content | trim != (deployment_version ~ '|' ~ deployment_token ~ '|' ~ inventory_hostname)
          no_log: true
          when: not ansible_check_mode
        - assert:
            that: inventory_hostname != failure_host
          when: not ansible_check_mode
      rescue:
        - copy:
            content: '{{ previous_configuration.stdout }}'
            dest: /srv/rustsible/rollout/config
          no_log: true
          notify: reload deployed configuration
        - meta: flush_handlers
        - copy:
            content: rescued
            dest: /srv/rustsible/rollout/rescued
            mode: '0600'
      always:
        - stat:
            path: /srv/rustsible/rollout/config
            checksum_algorithm: sha256
          register: installed
        - assert:
            that: installed.stat.mode == '0755'
        - set_fact:
            batch_done: true
  handlers:
    - name: reload deployed configuration
      command:
        argv: [cp, /srv/rustsible/rollout/config, /srv/rustsible/rollout/applied]
      no_log: true
YAML
release_snapshot() {
    for server in "${rollout_containers[@]}"; do
        docker exec "$server" sh -eu -c '
            find /srv/rustsible/rollout -type f -exec sha256sum {} \;
            find /srv/rustsible/rollout -exec stat -c "%n:%a:%u:%g" {} \;
        '
    done
}
release_options=(-i "$fixture_dir/rollout-inventory" --collections-path "$fixture_dir/collections" --vault-id "release@$fixture_dir/vault-password")
release_snapshot > "$fixture_dir/release-before"
rustsible playbook "$fixture_dir/release.yml" "${release_options[@]}" --check --diff > "$fixture_dir/release-check.log"
release_snapshot > "$fixture_dir/release-after"
diff -u "$fixture_dir/release-before" "$fixture_dir/release-after"
rustsible playbook "$fixture_dir/release.yml" "${release_options[@]}" --diff > "$fixture_dir/release-first.log"
rustsible playbook "$fixture_dir/release.yml" "${release_options[@]}" --diff > "$fixture_dir/release-second.log"
[[ $(grep -c 'changed=0 unreachable=0 failed=0' "$fixture_dir/release-second.log") == 3 ]]
for index in 0 1 2; do
    server=${rollout_containers[$index]}; user=${users[$index]}; name=node$((index + 1))
    docker exec "$server" sh -eu -c '
        [ "$(cat /srv/rustsible/rollout/config)" = "release-v1|release-secret-canary|$1" ]
        cmp /srv/rustsible/rollout/config /srv/rustsible/rollout/applied
        [ "$(sha256sum /srv/rustsible/rollout/artifact | cut -d " " -f 1)" = "$3" ]
        [ "$(stat -c %U:%a /srv/rustsible/rollout/config)" = "$2:755" ]
        [ "$(stat -c %U:%a /srv/rustsible/rollout/artifact)" = "$2:755" ]
        [ -z "$(find /tmp -name ".rustsible-worker-*")" ]
        ! command -v python3
        ! command -v curl
    ' sh "$name" "$user" "$payload_sha"
done
# Failure happens after deployment and the real reload. Rescue must restore the
# previous protected config, reload it, and let the remaining host finish.
rustsible playbook "$fixture_dir/release.yml" "${release_options[@]}" -e deployment_version=release-v2 -e failure_host=node2 > "$fixture_dir/release-rescue.log"
for index in 0 1 2; do
    server=${rollout_containers[$index]}; name=node$((index + 1)); version=release-v2
    if (( index == 1 )); then version=release-v1; fi
    docker exec "$server" sh -eu -c '
        [ "$(cat /srv/rustsible/rollout/config)" = "$2|release-secret-canary|$1" ]
        # Command output retains its newline, so rollback restores exact bytes.
        cmp /srv/rustsible/rollout/config /srv/rustsible/rollout/applied
        if [ "$1" = node2 ]; then [ "$(cat /srv/rustsible/rollout/rescued)" = rescued ]; else [ ! -e /srv/rustsible/rollout/rescued ]; fi
    ' sh "$name" "$version"
done
cat > "$fixture_dir/background.yml" <<EOF
- hosts: rollout
  gather_facts: false
  tasks:
    - debug:
        msg: '{{ deployment_token }}'
      no_log: true
    - example.native.probe:
        path: /srv/rustsible/rollout/config
        delay_ms: 500
      async: 30
      poll: 0
      no_log: true
      register: job
    - copy:
        content: '{{ job.ansible_job_id }}'
        dest: $fixture_dir/job-{{ inventory_hostname }}
      delegate_to: localhost
      no_log: true
EOF
rustsible playbook "$fixture_dir/background.yml" "${release_options[@]}" > "$fixture_dir/release-background.log"
mkdir "$fixture_dir/query"
cp "$fixture_dir/rollout-inventory" "$fixture_dir/query/inventory"
printf 'jobs:\n' > "$fixture_dir/query/jobs.yml"
for name in node1 node2 node3; do printf '  %s: "%s"\n' "$name" "$(cat "$fixture_dir/job-$name")" >> "$fixture_dir/query/jobs.yml"; done
cat > "$fixture_dir/query/status.yml" <<'YAML'
- hosts: rollout
  gather_facts: false
  vars_files: [jobs.yml]
  tasks:
    - async_status:
        jid: '{{ jobs[inventory_hostname] }}'
      register: finished
      until: finished is finished
      retries: 30
      delay: 0.1
    - assert:
        that: finished.probe.exists
    - async_status:
        jid: '{{ jobs[inventory_hostname] }}'
        mode: cleanup
YAML
rustsible playbook "$fixture_dir/query/status.yml" -i "$fixture_dir/query/inventory" > "$fixture_dir/release-query.log"
for server in "${rollout_containers[@]}"; do
    docker exec "$server" sh -eu -c '
        [ -z "$(find /tmp -name ".rustsible-worker-*")" ]
        for job_home in /root /home/deploy /home/app; do
            if [ -d "$job_home/.rustsible_async" ]; then [ -z "$(ls -A "$job_home/.rustsible_async")" ]; fi
        done
    '
done
if grep -Eq 'release-secret-canary|release-vault-password' "$fixture_dir"/release-*.log; then echo 'Protected release input leaked into output' >&2; exit 1; fi
printf 'Release deployment scenario passed: native %s/%s, three SSH servers, identities, checksum, roles, Vault, handlers, rolling, health, rescue, async and cleanup\n' "$target_arch" "$libc"
