# Each server has a different port, hostname and execution identity. Cleanup
# receives only container IDs created by this fixture, including partial setup.
rollout_containers=("$container")
for _ in 2 3; do
    extra=$(docker run -d --rm -p 127.0.0.1::22 "rustsible-ssh-fixture:$mode-$libc")
    additional_containers+=("$extra")
    rollout_containers+=("$extra")
done
printf '[rollout]\n' > "$fixture_dir/rollout-inventory"
users=(deploy root app)
for index in 0 1 2; do
    server=${rollout_containers[$index]}
    user=${users[$index]}
    [[ $(docker exec "$server" uname -m) == "$target_arch" ]]
    docker cp "$fixture_dir/key.pub" "$server:/tmp/authorized_keys" >/dev/null
    docker exec "$server" sh -eu -c '
        user=$1
        install -d -m 0700 -o deploy -g deploy /home/deploy/.ssh
        install -m 0600 -o deploy -g deploy /tmp/authorized_keys /home/deploy/.ssh/authorized_keys
        install -d -m 0755 -o "$user" -g "$user" /srv/rustsible/rollout
        printf original > /srv/rustsible/rollout/config
        chmod 0755 /srv/rustsible/rollout/config
        chown "$user:$user" /srv/rustsible/rollout/config
    ' sh "$user"
    for _ in {1..100}; do
        if docker exec "$server" test -f /run/sshd.pid; then break; fi
        sleep 0.1
    done
    server_port=$(docker port "$server" 22/tcp)
    server_port=${server_port##*:}
    docker exec "$server" sh -c 'cat /etc/ssh/ssh_host_*_key.pub' |
        while IFS= read -r public_key; do
            printf '[127.0.0.1]:%s %s\n' "$server_port" "$public_key"
        done >> "$fixture_dir/known_hosts"
    become=true
    if [[ $user == deploy ]]; then become=false; fi
    printf 'node%s ansible_connection=ssh ansible_host=127.0.0.1 ansible_port=%s ansible_user=deploy ansible_become=%s ansible_become_user=%s\n' \
        "$((index + 1))" "$server_port" "$become" "$user" >> "$fixture_dir/rollout-inventory"
done
cat >> "$fixture_dir/rollout-inventory" <<EOF
[all:vars]
ansible_ssh_private_key_file=$fixture_dir/key
rustsible_known_hosts_file=$fixture_dir/known_hosts
EOF
