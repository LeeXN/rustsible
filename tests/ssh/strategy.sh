source "$repo_dir/tests/ssh/rollout-setup.sh"
rollout_snapshot() {
    for server in "${rollout_containers[@]}"; do
        docker exec "$server" sh -eu -c '
            find /srv/rustsible/rollout -type f -exec sha256sum {} \;
            find /srv/rustsible/rollout -exec stat -c "%n:%a:%u:%g" {} \;
        '
    done
}
cat > "$fixture_dir/rolling.yml" <<'EOF'
- hosts: rollout
  gather_facts: false
  serial: [1, 2]
  throttle: 2
  pre_tasks:
    - debug:
        msg: '{{ inventory_hostname }}'
      failed_when: inventory_hostname != 'node1' and not (hostvars.node1.batch_done | default(false))
  tasks:
    - command: /bin/echo '{{ inventory_hostname }}'
      run_once: true
      changed_when: false
      register: leader
    - copy:
        content: '{{ inventory_hostname }}|{{ leader.stdout }}|{{ ansible_play_batch | join(",") }}'
        dest: /srv/rustsible/rollout/config
      notify: finished
    - command: cat /srv/rustsible/rollout/config
      register: readback
      changed_when: false
      failed_when: readback.stdout != (inventory_hostname ~ '|' ~ leader.stdout ~ '|' ~ (ansible_play_batch | join(',')))
  handlers:
    - name: finished
      copy:
        content: handled
        dest: /srv/rustsible/rollout/handler
        mode: '0600'
  post_tasks:
    - set_fact:
        batch_done: true
EOF
rollout_snapshot > "$fixture_dir/rolling-before"
rustsible playbook "$fixture_dir/rolling.yml" -i "$fixture_dir/rollout-inventory" --check > "$fixture_dir/rolling-check.log"
rollout_snapshot > "$fixture_dir/rolling-after"
diff -u "$fixture_dir/rolling-before" "$fixture_dir/rolling-after"
rustsible playbook "$fixture_dir/rolling.yml" -i "$fixture_dir/rollout-inventory" > "$fixture_dir/rolling.log"
rustsible playbook "$fixture_dir/rolling.yml" -i "$fixture_dir/rollout-inventory" > "$fixture_dir/rolling-second.log"
[[ $(grep -c 'changed=0 unreachable=0 failed=0' "$fixture_dir/rolling-second.log") == 3 ]]
for index in 0 1 2; do
    server=${rollout_containers[$index]}
    name=node$((index + 1))
    leader=node2
    batch=node2,node3
    if (( index == 0 )); then leader=node1; batch=node1; fi
    docker exec "$server" sh -eu -c '
        [ "$(cat /srv/rustsible/rollout/config)" = "$1|$2|$3" ]
        [ "$(stat -c %U /srv/rustsible/rollout/config)" = "$4" ]
        [ "$(stat -c %a /srv/rustsible/rollout/config)" = 755 ]
        [ "$(cat /srv/rustsible/rollout/handler)" = handled ]
    ' sh "$name" "$leader" "$batch" "${users[$index]}"
done
cat > "$fixture_dir/delegation.yml" <<EOF
- hosts: node1
  gather_facts: false
  tasks:
    - command: id -un
      delegate_to: '{{ item }}'
      loop: [node2, node3]
      register: identities
    - copy:
        content: '{{ inventory_hostname }}|{{ identities.results | selectattr("stdout", "defined") | map(attribute="stdout") | join(",") }}'
        dest: $fixture_dir/identities
      delegate_to: localhost
    - setup:
        filter: ansible_hostname
      delegate_to: node2
      delegate_facts: true
    - copy:
        content: '{{ hostvars.node2.ansible_facts.hostname }}|{{ hostvars.node1.ansible_facts.hostname | default("unset") }}'
        dest: $fixture_dir/delegated-facts
      delegate_to: localhost
    - command: /bin/false
      delegate_to: node2
- hosts: node2
  gather_facts: false
  tasks:
    - copy:
        content: target-remains-healthy
        dest: /srv/rustsible/rollout/healthy
- hosts: node1
  gather_facts: false
  tasks:
    - copy:
        content: must-not-run
        dest: /srv/rustsible/rollout/must-not-run
EOF
rollout_snapshot > "$fixture_dir/delegation-before"
rustsible playbook "$fixture_dir/delegation.yml" -i "$fixture_dir/rollout-inventory" --check > "$fixture_dir/delegation-check.log"
rollout_snapshot > "$fixture_dir/delegation-after"
diff -u "$fixture_dir/delegation-before" "$fixture_dir/delegation-after"
[[ ! -e $fixture_dir/identities && ! -e $fixture_dir/delegated-facts ]]
if rustsible playbook "$fixture_dir/delegation.yml" -i "$fixture_dir/rollout-inventory" > "$fixture_dir/delegation.log" 2>&1; then
    echo 'Delegated failure unexpectedly succeeded' >&2; exit 1
fi
[[ $(cat "$fixture_dir/identities") == 'node1|root,app' ]]
target_hostname=$(docker exec "${rollout_containers[1]}" hostname)
[[ $(cat "$fixture_dir/delegated-facts") == "$target_hostname|unset" ]]
docker exec "${rollout_containers[1]}" sh -eu -c '
    [ "$(cat /srv/rustsible/rollout/healthy)" = target-remains-healthy ]
    [ "$(stat -c %U /srv/rustsible/rollout/healthy)" = root ]
'
cat > "$fixture_dir/free.yml" <<'EOF'
- hosts: rollout
  gather_facts: false
  strategy: free
  tasks:
    - shell: 'if [ "{{ inventory_hostname }}" = node1 ]; then n=0; while [ ! -f /srv/rustsible/rollout/free-signal ]; do n=$((n+1)); [ "$n" -le 1000 ] || exit 41; sleep 0.01; done; fi'
      delegate_to: node1
      timeout: 15
    - set_fact:
        published: from-node2
      when: inventory_hostname == 'node2'
    - copy:
        content: ready
        dest: /srv/rustsible/rollout/free-signal
      delegate_to: node1
      when: inventory_hostname == 'node2'
    - copy:
        content: '{{ hostvars.node2.published | default("pending") }}'
        dest: /srv/rustsible/rollout/free-observed
      when: inventory_hostname in ['node1', 'node2']
    - copy:
        content: completed
        dest: /srv/rustsible/rollout/free-completed
      throttle: 2
      notify: free-finished
  handlers:
    - name: free-finished
      copy:
        content: handled
        dest: /srv/rustsible/rollout/free-handler
EOF
rollout_snapshot > "$fixture_dir/free-before"
rustsible playbook "$fixture_dir/free.yml" -i "$fixture_dir/rollout-inventory" --check > "$fixture_dir/free-check.log"
rollout_snapshot > "$fixture_dir/free-after"
diff -u "$fixture_dir/free-before" "$fixture_dir/free-after"
rustsible playbook "$fixture_dir/free.yml" -i "$fixture_dir/rollout-inventory" > "$fixture_dir/free.log"
for index in 0 1 2; do
    docker exec "${rollout_containers[$index]}" sh -eu -c '
        [ "$(cat /srv/rustsible/rollout/free-completed)" = completed ]
        [ "$(stat -c %U /srv/rustsible/rollout/free-completed)" = "$1" ]
        [ "$(cat /srv/rustsible/rollout/free-handler)" = handled ]
    ' sh "${users[$index]}"
    if (( index < 2 )); then
        docker exec "${rollout_containers[$index]}" sh -eu -c '[ "$(cat /srv/rustsible/rollout/free-observed)" = from-node2 ]'
    fi
done
for server in "${rollout_containers[@]}"; do
    docker exec "$server" sh -eu -c '
        [ ! -e /srv/rustsible/rollout/must-not-run ]
        [ -z "$(find /tmp -type d -name ".rustsible-worker-*")" ]
    '
done
printf 'SSH rolling/delegation passed (%s/%s): three servers, serial barriers, run_once, identity/owner, facts, logical failures, free/shared-target workers, idempotency, check and cleanup\n' "$target_arch" "$libc"
