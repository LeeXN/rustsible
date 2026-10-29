cat > "$fixture_dir/launch.yml" <<EOF
- hosts: all
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  tasks:
    - shell: 'while [ ! -f /srv/rustsible/{{ inventory_hostname }}/async-gate ]; do sleep 0.05; done; id -un > /srv/rustsible/{{ inventory_hostname }}/async-identity; printf ASYNC-{{ inventory_hostname }} > /srv/rustsible/{{ inventory_hostname }}/async-business; printf done'
      async: 120
      poll: 0
      register: background
    - copy:
        content: '{{ background.ansible_job_id }}'
        dest: $fixture_dir/jid-{{ inventory_hostname }}
      delegate_to: localhost
      when: background is not skipped
EOF
async_snapshot() {
    docker exec "$container" sh -eu -c '
        find /srv/rustsible -type f -exec sha256sum {} \;
        find /srv/rustsible -exec stat -c "%n:%a:%u:%g" {} \;
    '
}
async_snapshot > "$fixture_dir/async-before"
rustsible playbook "$fixture_dir/launch.yml" -i "$fixture_dir/inventory" --check > "$fixture_dir/async-check.log"
async_snapshot > "$fixture_dir/async-after"
diff -u "$fixture_dir/async-before" "$fixture_dir/async-after"
docker exec "$container" sh -eu -c '
    for home in /home/deploy /home/app /root; do [ ! -e "$home/.rustsible_async" ]; done
'
rustsible playbook "$fixture_dir/launch.yml" -i "$fixture_dir/inventory" > "$fixture_dir/async-launch.log"
printf 'jobs:\n' > "$fixture_dir/jobs.yml"
for pair in plain:deploy root:root app:app; do
    name=${pair%:*}; user=${pair#*:}; home=/home/$user
    if [[ $user == root ]]; then home=/root; fi
    jid=$(cat "$fixture_dir/jid-$name")
    [[ $jid =~ ^j[0-9a-f]{64}$ ]]
    printf '  %s: "%s"\n' "$name" "$jid" >> "$fixture_dir/jobs.yml"
    docker exec "$container" sh -eu -c '
        base=$1/.rustsible_async; job=$base/$2; user=$3; name=$4
        [ "$(stat -c %a:%U "$base")" = "700:$user" ]
        [ "$(stat -c %a:%U "$job")" = "700:$user" ]
        [ "$(stat -c %a:%U "$job/status.json")" = "600:$user" ]
        [ "$(stat -c %a:%U "$job/worker")" = "700:$user" ]
        [ ! -e "/srv/rustsible/$name/async-business" ]
        grep -q "\"started\":true" "$job/status.json"
        [ -z "$(find /tmp -type d -name ".rustsible-worker-*")" ]
    ' sh "$home" "$jid" "$user" "$name"
done
app_jid=$(cat "$fixture_dir/jid-app")
if rustsible ad-hoc root -i "$fixture_dir/inventory" -e ansible_host=127.0.0.1 -m async_status -a "jid=$app_jid" > "$fixture_dir/async-isolation.log" 2>&1; then
    echo 'A root-identity query unexpectedly found another identity job' >&2; exit 1
fi
cat > "$fixture_dir/query.yml" <<EOF
- hosts: all
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  vars_files: [$fixture_dir/jobs.yml]
  tasks:
    - file:
        path: /srv/rustsible/{{ inventory_hostname }}/async-gate
        state: touch
    - async_status:
        jid: '{{ jobs[inventory_hostname] }}'
      register: completed
      until: completed is finished
      retries: 120
      delay: 0.05
    - copy:
        content: '{{ completed.stdout }}|{{ completed.rc }}'
        dest: /srv/rustsible/{{ inventory_hostname }}/async-result
    - async_status:
        jid: '{{ jobs[inventory_hostname] }}'
        mode: cleanup
    - shell: 'printf polled > /srv/rustsible/{{ inventory_hostname }}/polled-business; printf final'
      async: 30
      poll: 1
      register: polled
    - copy:
        content: '{{ polled.stdout }}'
        dest: /srv/rustsible/{{ inventory_hostname }}/polled-result
    - file:
        path: /srv/rustsible/{{ inventory_hostname }}/native-async-file
        state: touch
        mode: '0600'
      async: 30
      poll: 1
    - lineinfile:
        path: /srv/rustsible/{{ inventory_hostname }}/native-async-file
        line: native
      async: 30
      poll: 1
    - command: /bin/echo ASYNC_PRIVATE_SENTINEL
      async: 30
      poll: 0
      no_log: true
      register: private_job
    - async_status:
        jid: '{{ private_job.ansible_job_id }}'
      register: private_result
      until: private_result is finished
      retries: 120
      delay: 0.05
    - copy:
        content: '{{ private_result.stdout }}'
        dest: /srv/rustsible/{{ inventory_hostname }}/private-result
        mode: '0600'
      no_log: true
    - async_status:
        jid: '{{ private_job.ansible_job_id }}'
        mode: cleanup
EOF
rustsible playbook "$fixture_dir/query.yml" -i "$fixture_dir/inventory" > "$fixture_dir/async-query.log"
if grep -q ASYNC_PRIVATE_SENTINEL "$fixture_dir/async-query.log"; then echo 'Async no_log output leaked' >&2; exit 1; fi
for pair in plain:deploy root:root app:app; do
    name=${pair%:*}; user=${pair#*:}; home=/home/$user
    if [[ $user == root ]]; then home=/root; fi
    docker exec "$container" sh -eu -c '
        base=/srv/rustsible/$1; user=$2; home=$3
        [ "$(cat "$base/async-business")" = "ASYNC-$1" ]
        [ "$(cat "$base/async-identity")" = "$user" ]
        [ "$(stat -c %U "$base/async-business")" = "$user" ]
        [ "$(cat "$base/async-result")" = "done|0" ]
        [ "$(cat "$base/polled-business")" = polled ]
        [ "$(cat "$base/polled-result")" = final ]
        [ "$(cat "$base/native-async-file")" = native ]
        [ "$(stat -c %a:%U "$base/native-async-file")" = "600:$user" ]
        [ "$(cat "$base/private-result")" = ASYNC_PRIVATE_SENTINEL ]
        [ -z "$(find "$home/.rustsible_async" -mindepth 1 -maxdepth 1)" ]
    ' sh "$name" "$user" "$home"
done
timeout_script="setsid /bin/sh -c 'echo \$\$ > /srv/rustsible/plain/timeout-pid; sleep 60' & wait"
rustsible ad-hoc plain -i "$fixture_dir/inventory" -e ansible_host=127.0.0.1 -m shell -a "$timeout_script" -B 2 -P 0 > "$fixture_dir/async-timeout-launch.log"
timeout_jid=$(sed -n 's/.*"ansible_job_id": "\(j[0-9a-f]*\)".*/\1/p' "$fixture_dir/async-timeout-launch.log" | head -n 1)
[[ $timeout_jid =~ ^j[0-9a-f]{64}$ ]]
for _ in {1..100}; do
    if rustsible ad-hoc plain -i "$fixture_dir/inventory" -e ansible_host=127.0.0.1 -m async_status -a "jid=$timeout_jid" > "$fixture_dir/async-timeout-query.log" 2>&1; then :; fi
    if grep -q '"finished": true' "$fixture_dir/async-timeout-query.log"; then break; fi
    sleep 0.05
done
grep -q '"timed_out": true' "$fixture_dir/async-timeout-query.log"
docker exec "$container" sh -eu -c 'pid=$(cat /srv/rustsible/plain/timeout-pid); [ ! -e "/proc/$pid" ]'
rustsible ad-hoc plain -i "$fixture_dir/inventory" -e ansible_host=127.0.0.1 -m async_status -a "jid=$timeout_jid mode=cleanup" > "$fixture_dir/async-timeout-cleanup.log"
if rustsible ad-hoc plain -i "$fixture_dir/inventory" -e ansible_host=127.0.0.1 -e ansible_async_dir=/noexec/async-private -m command -a 'touch /srv/rustsible/plain/noexec-must-not-run' -B 10 -P 0 > "$fixture_dir/async-noexec.log" 2>&1; then
    echo 'Async execution on a noexec cache unexpectedly succeeded' >&2; exit 1
fi
grep -q noexec "$fixture_dir/async-noexec.log"
docker exec "$container" sh -eu -c '
    [ ! -e /srv/rustsible/plain/noexec-must-not-run ]
    [ -z "$(find /tmp -type d -name ".rustsible-worker-*")" ]
'
printf 'SSH native async passed (%s/%s): disconnect survival, three identities/private state, query isolation, poll/cleanup, native files, no_log, detached deadline and noexec\n' "$target_arch" "$libc"
