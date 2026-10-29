printf 'vault-master-fixture\n' > "$fixture_dir/vault-password"
chmod 0600 "$fixture_dir/vault-password"
printf 'deploy:native-test-password\n' | docker exec -i "$container" chpasswd
docker exec "$container" sh -eu -c '
    printf "deploy ALL=(ALL) ALL\n" > /etc/sudoers.d/deploy
    chmod 0440 /etc/sudoers.d/deploy
    printf "\nPasswordAuthentication yes\n" >> /etc/ssh/sshd_config
    kill -HUP "$(cat /run/sshd.pid)"
'
mkdir -p "$fixture_dir/group_vars" "$fixture_dir/roles/protected/tasks" "$fixture_dir/roles/protected/templates"
cat > "$fixture_dir/group_vars/all.yml" <<'YAML'
ansible_password: native-test-password
ansible_become_password: native-test-password
vault_token: protected-config-canary
YAML
cat > "$fixture_dir/roles/protected/templates/config.j2" <<'EOF_TEMPLATE'
{{ vault_token }}:{{ inventory_hostname }}
EOF_TEMPLATE
cat > "$fixture_dir/roles/protected/tasks/main.yml" <<'YAML'
- template:
    src: config.j2
    dest: /srv/rustsible/{{ inventory_hostname }}/vault-config
    mode: '0600'
  no_log: true
  notify: finish
YAML
cat > "$fixture_dir/vault.yml" <<'YAML'
- hosts: all
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  roles: [protected]
  handlers:
    - name: finish
      copy:
        content: completed
        dest: /srv/rustsible/{{ inventory_hostname }}/vault-handler
YAML
for file in "$fixture_dir/group_vars/all.yml" "$fixture_dir/roles/protected/templates/config.j2" "$fixture_dir/roles/protected/tasks/main.yml" "$fixture_dir/vault.yml"; do
    "$controller_binary" vault encrypt "$file" --vault-id "prod@$fixture_dir/vault-password"
done
vault_snapshot() {
    docker exec "$container" sh -eu -c '
        find /srv/rustsible -type f -exec sha256sum {} \;
        find /srv/rustsible -exec stat -c "%n:%a:%u:%g" {} \;
    '
}
vault_snapshot > "$fixture_dir/vault-before"
printf 'wrong-password\n' > "$fixture_dir/wrong-password"
if rustsible playbook "$fixture_dir/vault.yml" -i "$fixture_dir/inventory" --vault-password-file "$fixture_dir/wrong-password" > "$fixture_dir/vault-wrong.log" 2>&1; then echo 'Wrong Vault password unexpectedly succeeded' >&2; exit 1; fi
rustsible playbook "$fixture_dir/vault.yml" -i "$fixture_dir/inventory" --vault-id "prod@$fixture_dir/vault-password" --check --diff -vvv > "$fixture_dir/vault-check.log" 2>&1
vault_snapshot > "$fixture_dir/vault-after"
diff -u "$fixture_dir/vault-before" "$fixture_dir/vault-after"
printf 'vault-master-fixture\n' | "$prompt_fixture" "$controller_binary" --worker-dir "$worker_dir" playbook "$fixture_dir/vault.yml" -i "$fixture_dir/inventory" --vault-id prod@prompt --diff -vvv > "$fixture_dir/vault-first.log"
rustsible playbook "$fixture_dir/vault.yml" -i "$fixture_dir/inventory" --vault-id "prod@$fixture_dir/vault-password" --diff > "$fixture_dir/vault-second.log"
if grep -Eq 'protected-config-canary|vault-master-fixture|native-test-password' "$fixture_dir/vault-first.log" "$fixture_dir/vault-check.log" "$fixture_dir/vault-wrong.log"; then echo 'Vault plaintext leaked into diagnostics/diff' >&2; exit 1; fi
if grep -Eq 'changed=[1-9]' "$fixture_dir/vault-second.log"; then echo 'Vault deployment was not idempotent' >&2; exit 1; fi
for pair in plain:deploy root:root app:app; do
    name=${pair%:*}; user=${pair#*:}
    docker exec "$container" sh -eu -c '
        [ "$(cat "/srv/rustsible/$1/vault-config")" = "protected-config-canary:$1" ]
        [ "$(stat -c %U "/srv/rustsible/$1/vault-config")" = "$2" ]
        [ "$(stat -c %a "/srv/rustsible/$1/vault-config")" = 600 ]
        [ "$(cat "/srv/rustsible/$1/vault-handler")" = completed ]
        [ -z "$(find /tmp -type d -name ".rustsible-worker-*")" ]
    ' sh "$name" "$user"
done
# Run the released controller inside the same minimal target too. This is real
# no-Python execution of Vault, including protection metadata and IO failure.
docker cp "$controller_binary" "$container:/usr/local/bin/rustsible-vault-fixture" >/dev/null
docker cp "$fixture_dir/vault-password" "$container:/srv/vault-password" >/dev/null
docker exec "$container" sh -eu -c '
    ! command -v python3
    ! command -v curl
    chmod 0755 /usr/local/bin/rustsible-vault-fixture
    chmod 0600 /srv/vault-password
    printf protected-original > /srv/vault-capfile
    chmod 0755 /srv/vault-capfile
    setcap cap_net_bind_service=ep /srv/vault-capfile
    rustsible-vault-fixture vault encrypt /srv/vault-capfile --vault-password-file /srv/vault-password
    getcap /srv/vault-capfile | grep -q cap_net_bind_service
    rustsible-vault-fixture vault decrypt /srv/vault-capfile --vault-password-file /srv/vault-password
    [ "$(cat /srv/vault-capfile)" = protected-original ]
    [ "$(stat -c %a /srv/vault-capfile)" = 755 ]
    getcap /srv/vault-capfile | grep -q cap_net_bind_service
    dd if=/dev/zero of=/small/vault-data bs=1024 count=24 2>/dev/null
    chmod 0755 /small/vault-data
    sha256sum /small/vault-data > /srv/vault-before-hash
    stat -c "%i:%a:%u:%g" /small/vault-data > /srv/vault-before-metadata
    if rustsible-vault-fixture vault encrypt /small/vault-data --vault-password-file /srv/vault-password >/srv/vault-full.log 2>&1; then exit 1; fi
    sha256sum -c /srv/vault-before-hash
    [ "$(stat -c "%i:%a:%u:%g" /small/vault-data)" = "$(cat /srv/vault-before-metadata)" ]
    [ -z "$(find /small -name ".rustsible-vault-*")" ]
'
# Controller source files now contain ciphertext only; deployment outputs live
# on the target. The sole plaintext key source is the explicitly supplied file.
if grep -R -l 'protected-config-canary\|native-test-password' "$fixture_dir/group_vars" "$fixture_dir/roles" "$fixture_dir/vault.yml"; then echo 'Implicit decrypted controller file remained' >&2; exit 1; fi
printf 'SSH Vault passed (%s/%s): encrypted roles/templates/inventory variables, hidden master prompt, three SSH/sudo identities, check/diff/no_log, idempotency, capabilities, disk-full preservation and no-Python controller\n' "$target_arch" "$libc"
