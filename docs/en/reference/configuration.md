[English / 简体中文](../../zh-CN/reference/configuration.md) · [Index / 目录](../index.md)

# Configuration and environment

<a id="selection"></a>

## File selection and precedence

Required --config FILE wins, then ANSIBLE_CONFIG, project ansible.cfg, ~/.ansible.cfg, /etc/ansible/ansible.cfg. Only the first selected file is loaded, not merged with all later files. Recognized environment settings override that file, then explicit CLI settings override matching defaults. Connection variables still override keyword/default connection values.

<a id="keys"></a>

## Supported keys

[defaults]: inventory (one file), worker_dir, roles_path, collections_path, forks (positive; default 5), verbosity (0–4), remote_user, private_key_file, timeout (positive SSH seconds), host_key_checking (default true). [privilege_escalation]: become, become_user, become_method (sudo only). Unknown sections/keys/invalid values fail with source context. Relative file paths resolve beside the configuration file; environment paths resolve from cwd.

<a id="environment"></a>

## Environment mapping

ANSIBLE_INVENTORY→inventory; ANSIBLE_FORKS→forks; ANSIBLE_ROLES_PATH→roles_path; ANSIBLE_COLLECTIONS_PATH→collections_path; ANSIBLE_VERBOSITY→verbosity; ANSIBLE_REMOTE_USER→remote_user; ANSIBLE_PRIVATE_KEY_FILE→private_key_file; ANSIBLE_TIMEOUT→timeout; ANSIBLE_HOST_KEY_CHECKING→host_key_checking; ANSIBLE_BECOME/ANSIBLE_BECOME_USER/ANSIBLE_BECOME_METHOD map to their privilege keys. ANSIBLE_CONFIG chooses a file, rather than an ordinary configuration key.

<a id="example"></a>

## Example file

```ini
[defaults]
inventory = inventory.yml
forks = 5
roles_path = roles
collections_path = collections
worker_dir = workers
host_key_checking = true

[privilege_escalation]
become = false
become_user = root
become_method = sudo
```

<a id="connection"></a>

## Effective connection variables

Use ansible_host/user/port/connection and ansible_ssh_private_key_file/password for actual connections; ansible_become/user/password/method govern become. rustsible_known_hosts_file and rustsible_host_key_checking select trust policy. rustsible_remote_tmp selects the target worker parent, rustsible_command_timeout the default module budget, ansible_async_dir protected job storage. Do not confuse ansible_become_user with ansible_user. Configuring collection paths authorizes declared native code.
