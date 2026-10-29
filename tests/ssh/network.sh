docker exec "$container" install -d -m 0755 /srv/http
docker cp "$repo_dir/tests/fixtures/tls/." "$container:/srv/http" >/dev/null
docker cp "$http_fixture" "$container:/usr/local/bin/rustsible-http-fixture" >/dev/null
docker exec "$container" chmod 0755 /usr/local/bin/rustsible-http-fixture
docker exec -d "$container" /usr/local/bin/rustsible-http-fixture 127.0.0.1:18080 /srv/http
docker exec -d "$container" /usr/local/bin/rustsible-http-fixture 127.0.0.1:18443 /srv/http --tls
cat > "$fixture_dir/network.yml" <<'EOF'
- hosts: all
  gather_facts: false
  vars:
    ansible_host: 127.0.0.1
  environment:
    HTTP_PROXY: http://127.0.0.1:18080
    http_proxy: http://127.0.0.1:18080
    NO_PROXY: localhost,127.0.0.1
    no_proxy: localhost,127.0.0.1
  tasks:
    - wait_for:
        host: 127.0.0.1
        port: 18080
        timeout: 5
        sleep: 0
    - uri:
        url: http://127.0.0.1:18080/echo
        method: POST
        headers:
          X-Host: '{{ inventory_hostname }}'
        body_format: json
        body:
          host: '{{ inventory_hostname }}'
        return_content: true
      register: posted
    - assert:
        that:
          - posted.status == 200
          - posted.json.method == 'POST'
          - posted.json.headers['x-host'] == inventory_hostname
          - inventory_hostname in posted.json.body
      when: posted is not skipped
    - uri:
        url: https://127.0.0.1:18443/hello
        ca_path: /srv/http/ca.pem
        return_content: true
      register: tls
    - assert:
        that: tls.content == 'hello'
      when: tls is not skipped
    - uri:
        url: http://must-not-resolve.invalid/hello
        return_content: true
      register: proxied
    - assert:
        that: proxied.content == 'hello'
      when: proxied is not skipped
    - uri:
        url: http://127.0.0.1:18080/auth
        url_username: user
        url_password: pass
      register: authenticated
    - assert:
        that: authenticated.status == 200
      when: authenticated is not skipped
    - uri:
        url: http://127.0.0.1:18080/eventually
        status_code: [200, 503]
      register: health
      until: health.status == 200
      retries: 10
      delay: 0.05
    - shell: 'sleep 0.1; printf READY > /srv/rustsible/{{ inventory_hostname }}/network-ready'
      async: 20
      poll: 0
      register: preparing
    - wait_for:
        path: /srv/rustsible/{{ inventory_hostname }}/network-ready
        search_regex: '^READY$'
        timeout: 5
        sleep: 0
    - async_status:
        jid: '{{ preparing.ansible_job_id }}'
      register: prepared
      until: prepared is finished
      retries: 50
      delay: 0.05
      when: preparing is not skipped
    - async_status:
        jid: '{{ preparing.ansible_job_id }}'
        mode: cleanup
      when: preparing is not skipped
    - copy:
        content: '{{ posted.json.method }}|{{ tls.content }}|{{ proxied.content }}|{{ health.status }}'
        dest: /srv/rustsible/{{ inventory_hostname }}/network-result
      when: posted is not skipped
EOF
network_snapshot() {
    docker exec "$container" sh -eu -c '
        find /srv/rustsible -type f -exec sha256sum {} \;
        find /srv/rustsible -exec stat -c "%n:%a:%u:%g" {} \;
    '
}
network_snapshot > "$fixture_dir/network-before"
rustsible playbook "$fixture_dir/network.yml" -i "$fixture_dir/inventory" --check > "$fixture_dir/network-check.log"
network_snapshot > "$fixture_dir/network-after"
diff -u "$fixture_dir/network-before" "$fixture_dir/network-after"
rustsible playbook "$fixture_dir/network.yml" -i "$fixture_dir/inventory" > "$fixture_dir/network.log"
if grep -q dXNlcjpwYXNz "$fixture_dir/network.log"; then echo 'URI authentication response leaked' >&2; exit 1; fi
for pair in plain:deploy root:root app:app; do
    name=${pair%:*}; user=${pair#*:}
    docker exec "$container" sh -eu -c '
        [ "$(cat "/srv/rustsible/$1/network-result")" = "POST|hello|hello|200" ]
        [ "$(cat "/srv/rustsible/$1/network-ready")" = READY ]
        [ "$(stat -c %U "/srv/rustsible/$1/network-result")" = "$2" ]
        [ -z "$(find /tmp -type d -name ".rustsible-worker-*")" ]
    ' sh "$name" "$user"
done
printf 'SSH wait_for/uri passed (%s/%s): three identities, target HTTP/TLS/CA/proxy, JSON/headers/auth, until readiness, bounded matching, check and cleanup\n' "$target_arch" "$libc"
