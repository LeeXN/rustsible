printf 'deploy:native-test-password\n' | docker exec -i "$container" chpasswd
docker exec "$container" sh -eu -c '
    printf "deploy ALL=(ALL) ALL\n" > /etc/sudoers.d/deploy
    chmod 0440 /etc/sudoers.d/deploy
    printf "\nPasswordAuthentication yes\n" >> /etc/ssh/sshd_config
    kill -HUP "$(cat /run/sshd.pid)"
'
sed '/ansible_ssh_private_key_file=/d' "$fixture_dir/inventory" > "$fixture_dir/password-inventory"
cat > "$fixture_dir/operator.yml" <<'YAML'
- hosts: all
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - name: Deploy selected configuration
      copy:
        content: visible-config
        dest: /srv/rustsible/{{ inventory_hostname }}/operator
      tags: deploy
      notify: mark
    - copy:
        content: hidden-diff-canary
        dest: /srv/rustsible/{{ inventory_hostname }}/operator-secret
      tags: deploy
      no_log: true
    - name: Must not execute excluded configuration
      copy:
        content: excluded
        dest: /srv/rustsible/{{ inventory_hostname }}/operator-excluded
      tags: excluded
    - assert:
        that:
          - ansible_password is undefined
          - ansible_become_password is undefined
      tags: deploy
  handlers:
    - name: mark
      copy:
        content: handled
        dest: /srv/rustsible/{{ inventory_hostname }}/operator-handler
      tags: never
YAML
operator_snapshot() {
    docker exec "$container" sh -eu -c '
        find /srv/rustsible -type f -exec sha256sum {} \;
        find /srv/rustsible -exec stat -c "%n:%a:%u:%g" {} \;
    '
}
operator_snapshot > "$fixture_dir/operator-before"
for flag in --syntax-check --list-hosts --list-tasks; do
    "$controller_binary" --worker-dir "$fixture_dir/missing-workers" playbook "$fixture_dir/operator.yml" -i "$fixture_dir/password-inventory" "$flag" --tags deploy -k -K > "$fixture_dir/operator-$flag.log"
done
if grep -q 'Must not execute excluded' "$fixture_dir/operator---list-tasks.log"; then echo 'Task listing ignored tags' >&2; exit 1; fi
operator_snapshot > "$fixture_dir/operator-after"
diff -u "$fixture_dir/operator-before" "$fixture_dir/operator-after"
run_operator_prompt() {
    printf 'native-test-password\nnative-test-password\n' | "$prompt_fixture" "$controller_binary" --worker-dir "$worker_dir" playbook "$fixture_dir/operator.yml" -i "$fixture_dir/password-inventory" --tags deploy --diff -k -K "$@"
}
run_operator_prompt --check > "$fixture_dir/operator-check.log"
operator_snapshot > "$fixture_dir/operator-after"
diff -u "$fixture_dir/operator-before" "$fixture_dir/operator-after"
run_operator_prompt > "$fixture_dir/operator-first.log"
run_operator_prompt > "$fixture_dir/operator-second.log"
if grep -Eq 'hidden-diff-canary|native-test-password' "$fixture_dir/operator-first.log"; then echo 'Credential or private diff leaked' >&2; exit 1; fi
if grep -Eq 'changed=[1-9]' "$fixture_dir/operator-second.log"; then echo 'Operator deployment was not idempotent' >&2; exit 1; fi
grep -q 'visible-config' "$fixture_dir/operator-first.log"
for pair in plain:deploy root:root app:app; do
    name=${pair%:*}; user=${pair#*:}
    docker exec "$container" sh -eu -c '
        [ "$(cat "/srv/rustsible/$1/operator")" = visible-config ]
        [ "$(cat "/srv/rustsible/$1/operator-handler")" = handled ]
        [ ! -e "/srv/rustsible/$1/operator-excluded" ]
        [ "$(stat -c %U "/srv/rustsible/$1/operator")" = "$2" ]
        [ -z "$(find /tmp -type d -name ".rustsible-worker-*")" ]
    ' sh "$name" "$user"
done
# CLI keys override configuration, while declared inventory keys still win.
rustsible ad-hoc plain -i "$fixture_dir/password-inventory" -e ansible_host=127.0.0.1 --private-key "$fixture_dir/key" -m command -a 'id -un' > "$fixture_dir/operator-key.log"
rustsible ad-hoc plain -i "$fixture_dir/inventory" -e ansible_host=127.0.0.1 --private-key "$fixture_dir/nonexistent-key" -m command -a 'id -un' > "$fixture_dir/operator-key-precedence.log"
grep -q 'deploy' "$fixture_dir/operator-key.log"
printf 'SSH operator CLI passed (%s/%s): hidden SSH/sudo prompts, three identities, private key precedence, tags/handlers, no_log diff, idempotency, check and no-connection inspections\n' "$target_arch" "$libc"
