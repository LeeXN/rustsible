cp -a "$repo_dir/tests/reference/reuse" "$fixture_dir/reuse"
while IFS= read -r -d '' file; do
    sed -i 's|@OUTPUT@|/srv/rustsible/{{ inventory_hostname }}/reuse|g; s/hosts: localhost/hosts: all/g' "$file"
done < <(find "$fixture_dir/reuse" -type f -print0)
sed -i '/  vars:/a\    ansible_host: 127.0.0.1' "$fixture_dir/reuse/main.yml"
docker exec "$container" sh -eu -c '
    for pair in plain:deploy root:root app:app; do
        name=${pair%:*}; user=${pair#*:}
        install -d -m 0755 -o "$user" -g "$user" "/srv/rustsible/$name/reuse"
    done
'
rustsible --config "$fixture_dir/reuse/ansible.cfg" playbook "$fixture_dir/reuse/main.yml" -i "$fixture_dir/inventory" --check > "$fixture_dir/reuse-check.log"
docker exec "$container" sh -eu -c '
    for name in plain root app; do [ -z "$(find "/srv/rustsible/$name/reuse" -type f)" ]; done
'
rustsible --config "$fixture_dir/reuse/ansible.cfg" playbook "$fixture_dir/reuse/main.yml" -i "$fixture_dir/inventory" > "$fixture_dir/reuse.log"
docker exec "$container" sh -eu -c '
    for pair in plain:deploy root:root app:app; do
        name=${pair%:*}; user=${pair#*:}; base=/srv/rustsible/$name/reuse
        [ "$(cat "$base/config")" = "default|role|dependency|parameter" ]
        [ "$(cat "$base/public")" = "default|role" ]
        [ "$(cat "$base/included")" = included ]
        [ "$(cat "$base/rescued")" = "deliberate failure|1" ]
        [ "$(cat "$base/order")" = "pre;handler-parameter;dependency;role;always;handler-parameter;post;" ]
        [ "$(stat -c %U "$base/config")" = "$user" ]
        [ ! -e "$base/must-not-run" ]
    done
    [ -z "$(find /tmp -type d -name ".rustsible-worker-*")" ]
'
printf 'SSH task reuse/recovery passed (%s/%s): three identities, imported sources, role dependencies/params/templates, handler phases/listen, dynamic conditions, rescue/always, check and cleanup\n' "$target_arch" "$libc"

cat > "$fixture_dir/inherited-inventory" <<EOF
node ansible_connection=ssh ansible_host=127.0.0.1 ansible_port=$port ansible_user=deploy ansible_ssh_private_key_file=$fixture_dir/key rustsible_known_hosts_file=$fixture_dir/known_hosts
EOF
cat > "$fixture_dir/inherited.yml" <<'EOF'
- hosts: all
  gather_facts: false
  become: true
  become_user: root
  tasks:
    - block:
        - command: id -un
          register: inherited_identity
        - copy:
            content: '{{ inherited_identity.stdout }}'
            dest: /srv/rustsible/app/reuse/inherited-identity
      become: true
      become_user: app
    - block:
        - command: id -un
          register: direct_identity
        - copy:
            content: '{{ direct_identity.stdout }}'
            dest: /srv/rustsible/plain/reuse/inherited-identity
      become: false
EOF
rustsible playbook "$fixture_dir/inherited.yml" -i "$fixture_dir/inherited-inventory" > "$fixture_dir/inherited.log"
docker exec "$container" sh -eu -c '
    [ "$(cat /srv/rustsible/app/reuse/inherited-identity)" = app ]
    [ "$(stat -c %U /srv/rustsible/app/reuse/inherited-identity)" = app ]
    [ "$(cat /srv/rustsible/plain/reuse/inherited-identity)" = deploy ]
    [ "$(stat -c %U /srv/rustsible/plain/reuse/inherited-identity)" = deploy ]
    [ -z "$(find /tmp -type d -name ".rustsible-worker-*")" ]
'
printf 'SSH lexical become inheritance passed (%s/%s): app block and explicit false override play root defaults\n' "$target_arch" "$libc"
