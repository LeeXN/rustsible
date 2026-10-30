expected_controller=$5
controller_info=$("$controller_binary" build-info)
[[ $(printf '%s' "$controller_info" | jq -r .arch) == "$expected_controller" ]]
[[ $expected_controller != "$target_arch" ]]
cat > "$fixture_dir/cross.yml" <<EOF
- hosts: all
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - assert:
        that: ansible_architecture == '$target_arch'
    - copy:
        content: 'controller=$expected_controller target={{ ansible_architecture }}'
        dest: /srv/rustsible/{{ inventory_hostname }}/cross
        mode: '0755'
    - stat:
        path: /srv/rustsible/{{ inventory_hostname }}/cross
        checksum_algorithm: sha256
      register: deployed
      when: not ansible_check_mode
    - assert:
        that:
          - deployed.stat.mode == '0755'
          - deployed.stat.checksum | length == 64
      when: not ansible_check_mode
EOF
rustsible playbook "$fixture_dir/cross.yml" -i "$fixture_dir/inventory" --check > "$fixture_dir/cross-check.log"
docker exec "$container" sh -eu -c 'for name in plain root app; do [ ! -e "/srv/rustsible/$name/cross" ]; done'
rustsible playbook "$fixture_dir/cross.yml" -i "$fixture_dir/inventory" > "$fixture_dir/cross-first.log"
rustsible playbook "$fixture_dir/cross.yml" -i "$fixture_dir/inventory" > "$fixture_dir/cross-second.log"
[[ $(grep -c 'changed=0 unreachable=0 failed=0' "$fixture_dir/cross-second.log") == 3 ]]
for pair in plain:deploy root:root app:app; do
    name=${pair%:*}; user=${pair#*:}
    docker exec "$container" sh -eu -c '
        [ "$(cat "/srv/rustsible/$1/cross")" = "controller=$3 target=$4" ]
        [ "$(stat -c %U:%a "/srv/rustsible/$1/cross")" = "$2:755" ]
        [ -z "$(find /tmp -name ".rustsible-worker-*")" ]
    ' sh "$name" "$user" "$expected_controller" "$target_arch"
done
printf 'Cross-architecture release passed: QEMU %s controller -> native %s SSH worker, three identities, check, idempotence and cleanup\n' "$expected_controller" "$target_arch"
if [[ -n ${RUSTSIBLE_ACCEPTANCE_OUTPUT:-} ]]; then
    evidence_dir=$RUSTSIBLE_ACCEPTANCE_OUTPUT/cross-$expected_controller-to-$target_arch
    mkdir -p "$evidence_dir"
    printf '%s\n' "$controller_info" > "$evidence_dir/controller.json"
    cp "$fixture_dir"/cross-*.log "$evidence_dir/"
    docker exec "$container" sh -eu -c '
        printf "native_target_arch="; uname -m
        for name in plain root app; do
            stat -c "%n:%u:%g:%a" "/srv/rustsible/$name/cross"
            sha256sum "/srv/rustsible/$name/cross"
        done
        printf "bootstrap_directories="; find /tmp -name ".rustsible-worker-*" | wc -l
    ' > "$evidence_dir/target.txt"
fi
