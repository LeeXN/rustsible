mkdir -p "$fixture_dir/vars"
docker exec "$container" sh -eu -c '
    name=$(uname -n)
    awk -v name="$name" '\''{ for (i=1;i<=NF;i++) if ($i == name) { print $0 " facts.rustsible.test"; next }; print }'\'' /etc/hosts > /tmp/fixture-hosts
    cat /tmp/fixture-hosts > /etc/hosts
    rm /tmp/fixture-hosts
'
cat > "$fixture_dir/vars/data.yml" <<'EOF'
configuration: file
number: '0012'
sequence: [first, second]
EOF
cat > "$fixture_dir/variables.yml" <<'EOF'
- hosts: all
  gather_subset: ['!all', '!min', platform]
  gather_timeout: 10
  vars:
    ansible_host: 127.0.0.1
  vars_files: [vars/data.yml]
  tasks:
    - setup:
        gather_subset: ['!all', '!min', architecture, distribution, network]
        filter: [ansible_architecture, ansible_os_family, ansible_interfaces]
    - include_vars:
        file: vars/data.yml
        name: loaded
    - set_fact:
        accumulated: '{{ accumulated | default([]) + [item] }}'
      loop: '{{ loaded.sequence }}'
    - copy:
        content: '{{ ansible_architecture }}|{{ ansible_os_family }}|{{ accumulated | join(",") }}|{{ number }}|{{ configuration }}|{{ loaded.configuration }}|{{ hostvars.root.ansible_architecture }}|{{ ansible_fqdn }}'
        dest: /srv/rustsible/{{ inventory_hostname }}/variables
EOF
rustsible playbook "$fixture_dir/variables.yml" -i "$fixture_dir/inventory" -e configuration=extra --check > "$fixture_dir/variables-check.log"
docker exec "$container" sh -eu -c 'for name in plain root app; do [ ! -e "/srv/rustsible/$name/variables" ]; done'
rustsible playbook "$fixture_dir/variables.yml" -i "$fixture_dir/inventory" -e configuration=extra > "$fixture_dir/variables.log"
family=Debian
[[ $libc != musl ]] || family=Alpine
docker exec "$container" sh -eu -c '
    for name in plain root app; do
        [ "$(cat "/srv/rustsible/$name/variables")" = "$1|$2|first,second|0012|extra|file|$1|facts.rustsible.test" ]
    done
' sh "$target_arch" "$family"
# An inventory connection variable outranks keywords; configuration defaults
# do not. Use a host without inventory become variables to exercise that layer.
printf '[all]\nnode\n' > "$fixture_dir/config-inventory"
sed -n '/^\[all:vars\]/,$p' "$fixture_dir/inventory" >> "$fixture_dir/config-inventory"
cat > "$fixture_dir/ansible.cfg" <<'EOF'
[privilege_escalation]
become=true
become_user=root
EOF
cat > "$fixture_dir/config-become.yml" <<'EOF'
- hosts: all
  gather_facts: false
  become: false
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - command: id -un
      register: identity
    - copy:
        content: '{{ identity.stdout }}'
        dest: /srv/rustsible/plain/config-become
- hosts: all
  gather_facts: false
  become: true
  become_user: app
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - command: id -un
      register: identity
    - copy:
        content: '{{ identity.stdout }}'
        dest: /srv/rustsible/app/config-become
- hosts: all
  gather_facts: false
  become: true
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - command: id -un
      become: false
      register: identity
    - copy:
        content: '{{ identity.stdout }}'
        dest: /srv/rustsible/plain/task-become
      become: false
EOF
rustsible --config "$fixture_dir/ansible.cfg" playbook "$fixture_dir/config-become.yml" -i "$fixture_dir/config-inventory" > "$fixture_dir/config-become.log"
docker exec "$container" sh -eu -c '
    [ "$(cat /srv/rustsible/plain/config-become)" = deploy ]
    [ "$(cat /srv/rustsible/app/config-become)" = app ]
    [ "$(cat /srv/rustsible/plain/task-become)" = deploy ]
    [ "$(stat -c %U /srv/rustsible/plain/config-become)" = deploy ]
    [ "$(stat -c %U /srv/rustsible/app/config-become)" = app ]
    [ -z "$(find /tmp -type d -name ".rustsible-worker-*")" ]
'
printf 'SSH variable acceptance passed (%s/%s): source files, typed values, include_vars/set_fact/hostvars, native fact filters, check invariance and config/play/task identity precedence\n' "$target_arch" "$libc"
