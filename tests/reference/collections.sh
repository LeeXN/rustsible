collection_root=$fixture_dir/collections/ansible_collections/example/declarative
mkdir -p "$collection_root/roles/app/tasks" "$collection_root/roles/app/defaults" "$collection_root/roles/app/templates" "$collection_root/playbooks" "$fixture_dir/collection-output"
printf 'listen_port: 8080\nrole_label: default\n' > "$collection_root/roles/app/defaults/main.yml"
printf '{{ role_label }}:{{ listen_port }}:{{ inventory_hostname }}\n' > "$collection_root/roles/app/templates/config.j2"
cat > "$collection_root/roles/app/tasks/main.yml" <<'YAML'
- template:
    src: config.j2
    dest: '{{ output }}/config'
    mode: '0600'
- include_tasks: verify.yml
YAML
cat > "$collection_root/roles/app/tasks/verify.yml" <<'YAML'
- assert:
    that:
      - listen_port == 9090
      - role_label == 'selected'
- copy:
    content: '{{ role_name }}|{{ ansible_role_name }}|{{ ansible_collection_name }}'
    dest: '{{ output }}/role-name'
YAML
cat > "$collection_root/playbooks/deploy.yml" <<'YAML'
- hosts: all
  gather_facts: false
  collections: [example.declarative]
  vars:
    listen_port: 9090
  roles:
    - role: example.declarative.app
      role_label: selected
YAML
ANSIBLE_COLLECTIONS_PATH="$fixture_dir/collections" uv tool run --from ansible-core==2.21.0 ansible-playbook -i "$fixture_dir/inventory" example.declarative.deploy -e "output=$fixture_dir/collection-output" > "$fixture_dir/reference-collection.log" 2>&1
mv "$fixture_dir/collection-output" "$fixture_dir/reference-collection"
mkdir "$fixture_dir/collection-output"
"$binary" playbook --collections-path "$fixture_dir/collections" -i "$fixture_dir/inventory" example.declarative.deploy -e "output=$fixture_dir/collection-output" > "$fixture_dir/native-collection.log" 2>&1
diff -r "$fixture_dir/reference-collection" "$fixture_dir/collection-output"
[[ $(cat "$fixture_dir/collection-output/role-name") == 'app|example.declarative.app|example.declarative' ]]
