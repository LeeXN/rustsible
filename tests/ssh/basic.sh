group_name=rustsible_fixture_group
cat > "$fixture_dir/group-check.yml" <<EOF
- hosts: all
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - group:
        name: $group_name
        gid: 38000
        system: true
EOF
docker exec "$container" sh -eu -c 'if getent group "$1"; then exit 1; fi' sh "$group_name"
rustsible playbook "$fixture_dir/group-check.yml" -i "$fixture_dir/inventory" --check > "$fixture_dir/group-check.log"
docker exec "$container" sh -eu -c 'if getent group "$1"; then exit 1; fi' sh "$group_name"
cat > "$fixture_dir/group.yml" <<EOF
- hosts: root
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - group:
        name: $group_name
        gid: 38000
        system: true
      register: created
    - command:
        argv: [getent, group, $group_name]
      register: created_state
      changed_when: false
    - group:
        name: $group_name
        gid: 38000
      register: repeated
    - assert:
        that:
          - created.changed
          - created.gid == 38000
          - "'$group_name:x:38000:' in created_state.stdout"
          - not repeated.changed
    - group:
        name: $group_name
        gid: 38001
      register: modified
    - command:
        argv: [getent, group, $group_name]
      register: modified_state
      changed_when: false
    - assert:
        that:
          - modified.changed
          - modified.gid == 38001
          - "'$group_name:x:38001:' in modified_state.stdout"
    - group:
        name: $group_name
        state: absent
      register: removed
    - group:
        name: $group_name
        state: absent
      register: absent
    - assert:
        that: [removed.changed, not absent.changed]
- hosts: all
  gather_facts: false
  vars:
    ansible_host: unreachable.invalid
  tasks:
    - block:
        - fail:
            msg: expected recovery
      rescue:
        - assert:
            that: ansible_failed_result.msg == 'expected recovery'
    - assert:
        that: true
        quiet: true
        success_msg: QUIET_PRIVATE_SENTINEL
EOF
rustsible playbook "$fixture_dir/group.yml" -i "$fixture_dir/inventory" > "$fixture_dir/group.log"
if grep -q QUIET_PRIVATE_SENTINEL "$fixture_dir/group.log"; then echo 'Quiet assertion message was displayed' >&2; exit 1; fi
docker exec "$container" sh -eu -c '
    if getent group "$1"; then exit 1; fi
    [ -z "$(find /tmp -type d -name ".rustsible-worker-*")" ]
' sh "$group_name"
cat > "$fixture_dir/invalid-group.yml" <<EOF
- hosts: root
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - group:
        name: $group_name
        gid: -1
EOF
if rustsible playbook "$fixture_dir/invalid-group.yml" -i "$fixture_dir/inventory" > "$fixture_dir/invalid-group.log" 2>&1; then
    echo 'Invalid group GID unexpectedly succeeded' >&2; exit 1
fi
docker exec "$container" sh -eu -c 'if getent group "$1"; then exit 1; fi' sh "$group_name"
printf 'SSH assert/fail/group passed (%s/%s): native group lifecycle/GIDs, idempotency, three-identity check, prevalidation, controller recovery/quiet and cleanup\n' "$target_arch" "$libc"
