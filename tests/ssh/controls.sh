cat > "$fixture_dir/controls.yml" <<'EOF'
- hosts: all
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
    destination: /srv/rustsible/{{ inventory_hostname }}
  environment:
    RUSTSIBLE_TEST_LEVEL: play
  tasks:
    - command:
        argv: [/bin/cat]
        stdin: "SSH literal; {{ inventory_hostname }}"
        stdin_add_newline: false
        chdir: '{{ destination }}'
      register: input_result
    - copy:
        content: '{{ input_result.stdout }}'
        dest: '{{ destination }}/stdin'
    - shell: 'n=$(cat {{ destination }}/count-{{ item }} 2>/dev/null || echo 0); n=$((n+1)); printf %s "$n" > {{ destination }}/count-{{ item }}; printf "%s|%s" "$n" "$RUSTSIBLE_TEST_LEVEL"; test "$n" -ge 2'
      environment:
        RUSTSIBLE_TEST_LEVEL: task
      register: attempt
      retries: 2
      delay: 0
      until: attempt.rc == 0
      changed_when: attempt.stdout == '2|task'
      loop: [one, two]
    - copy:
        content: '{{ attempt.results[0].attempts }}|{{ attempt.results[1].attempts }}|{{ attempt.results[0].stdout }}'
        dest: '{{ destination }}/attempts'
    - shell: 'test "$RUSTSIBLE_TEST_LEVEL" = play'
      changed_when: false
    - command: /bin/false
      failed_when: false
      changed_when: false
EOF
rustsible playbook "$fixture_dir/controls.yml" -i "$fixture_dir/inventory" > "$fixture_dir/controls.log"
docker exec "$container" sh -eu -c '
    for name in plain root app; do
        base=/srv/rustsible/$name
        [ "$(cat "$base/stdin")" = "SSH literal; $name" ]
        [ "$(cat "$base/attempts")" = "2|2|2|task" ]
        [ "$(cat "$base/count-one")" = 2 ]
        [ "$(cat "$base/count-two")" = 2 ]
    done
'
cat > "$fixture_dir/control-timeout.yml" <<'EOF'
- hosts: all
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
    destination: /srv/rustsible/{{ inventory_hostname }}
  tasks:
    - shell: |
        printf x >> {{ destination }}/invocations
        setsid sh -c 'echo $$ > {{ destination }}/child; sleep 30' &
        wait
      timeout: 1
      retries: 3
      delay: 0
      ignore_errors: true
    - command: touch {{ destination }}/later
EOF
if rustsible playbook "$fixture_dir/control-timeout.yml" -i "$fixture_dir/inventory" > "$fixture_dir/control-timeout.log" 2>&1; then
    echo 'Timed-out mutations were accepted' >&2; exit 1
fi
docker exec "$container" sh -eu -c '
    for name in plain root app; do
        base=/srv/rustsible/$name
        [ "$(cat "$base/invocations")" = x ]
        [ ! -e "$base/later" ]
        ! kill -0 "$(cat "$base/child")" 2>/dev/null
    done
    [ -z "$(find /tmp -type d -name ".rustsible-worker-*")" ]
'
printf 'SSH task controls passed (%s): three identities, argv/stdin/chdir, environment isolation, per-item retries/register, conditions, timeout tree cleanup and no uncertain replay\n' "$target_arch"
