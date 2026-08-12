# Rustsible 手工验收方案

本方案用于验收当前仓库声明支持的功能，而不是验证与完整 Ansible 的兼容性。
先执行本地无特权测试；SSH、`become`、用户、软件包和服务测试只应在可回滚的
一次性 Linux 虚拟机或容器中执行。

## 1. 验收环境与证据

建议准备：

- 控制机：当前稳定版 Rust、`cargo-audit`、`cargo-llvm-cov`；
- 本地目标：Linux/macOS 上的 `localhost`；
- 远程目标：至少两台一次性 Linux VM，用于 SSH、并发和 host pattern；
- 普通 SSH 用户、可用的 sudo 配置、已人工核验的 SSH host-key 指纹；
- 每一步保存命令、退出码、stdout/stderr 和目标机变更前后状态。

先构建并记录版本：

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --all-features --locked
cargo build --release --all-features --locked
target/release/rustsible --version
```

预期：所有命令退出码为 0，release 二进制位于
`target/release/rustsible`。

## 2. 本地核心验收

仓库提供了可直接执行的 fixture：

- inventory：`examples/manual/inventory-local.ini`；
- playbook：`examples/manual/local-smoke.yml`；
- 清理：`examples/manual/cleanup-local.yml`。

### 2.1 Inventory 与敏感值展示

```bash
target/release/rustsible inventory-debug \
  -i examples/manual/inventory-local.ini
```

预期：识别 `manual_local` 和 `localhost`，变量值只显示 `<redacted>`，不会输出
连接凭据或任意 inventory value。

再分别创建临时 INI/YAML inventory，验证：

- `all -> parent -> child -> host` 的变量优先级；
- typed YAML list/map/bool/number 保持类型；
- `[group:children]` 环、错误端口、错误布尔值、未知 section suffix 均明确失败；
- `all`、glob、并集、`&` 交集、`!` 排除和 `--limit` 结果符合预期。

### 2.2 全局 check mode 不落盘

```bash
target/release/rustsible playbook examples/manual/local-smoke.yml \
  -i examples/manual/inventory-local.ini --check -vvvv \
  > /tmp/rustsible-manual-check.log 2>&1
test ! -e /tmp/rustsible-manual
```

预期：playbook 成功，报告预测变更，但 `/tmp/rustsible-manual` 不存在。fixture
中故意设置了 task 级 `check_mode: false`，它也不能覆盖 CLI 全局 `--check`。

检查 no_log：

```bash
if rg -n "manual-secret-do-not-log" /tmp/rustsible-manual-check.log; then
  echo "FAIL: secret leaked"
else
  echo "PASS: secret redacted"
fi
```

预期：日志中找不到 secret，相关 task 输出为 censored/redacted。

### 2.3 首次执行与功能结果

```bash
target/release/rustsible playbook examples/manual/local-smoke.yml \
  -i examples/manual/inventory-local.ini -vvvv \
  > /tmp/rustsible-manual-first.log 2>&1

find /tmp/rustsible-manual -maxdepth 1 -type f -print -exec sed -n '1,4p' {} \;
```

逐项确认：

- `app.conf`、`managed-lines.conf`、`rendered.conf`、`secret.txt` 存在且权限正确；
- 中文和 emoji 完整，模板没有截断或 panic；
- SHA-512 hash 以 `$6$manualsalt$` 开头；
- `alpha.txt`、`beta.txt` 创建成功，register 中存在两个 `results`；
- handler 只在被通知的 localhost 上执行，并创建 `handler.marker`；
- 日志中仍不得出现 `manual-secret-do-not-log`。

### 2.4 第二次执行必须幂等

```bash
target/release/rustsible playbook examples/manual/local-smoke.yml \
  -i examples/manual/inventory-local.ini \
  > /tmp/rustsible-manual-second.log 2>&1
rg -n "changed=0" /tmp/rustsible-manual-second.log
```

预期：recap 为 `changed=0`，handler 不再运行，文件内容和 hash 不变化。

### 2.5 command 与 shell 边界

```bash
target/release/rustsible ad-hoc manual_local \
  -i examples/manual/inventory-local.ini \
  -m command -a "echo hello '>' /tmp/rustsible-command-redirection"
test ! -e /tmp/rustsible-command-redirection

target/release/rustsible ad-hoc manual_local \
  -i examples/manual/inventory-local.ini \
  -m shell -a "echo hello > /tmp/rustsible-shell-redirection"
test -s /tmp/rustsible-shell-redirection
```

预期：`command` 不解释重定向，`shell` 明确解释。对两者加 `--check` 时均跳过，
不执行命令。

### 2.6 错误传播与严格拒绝

```bash
target/release/rustsible ad-hoc manual_local \
  -i examples/manual/inventory-local.ini -m command -a "false"
echo $?

target/release/rustsible ad-hoc manual_local \
  -i examples/manual/inventory-local.ini -m definitely_not_a_module -a "x=1"
echo $?

target/release/rustsible playbook examples/playbooks/test_all_features.yml \
  -i examples/manual/inventory-local.ini
echo $?
```

预期：三者均返回非 0；未知模块和迁移 fixture 中不支持的关键字应在连接目标机前
报出明确错误，不能显示成功或静默忽略。

再验证：

```bash
target/release/rustsible playbook examples/manual/local-smoke.yml \
  -i examples/manual/inventory-local.ini --limit does-not-exist
echo $?

target/release/rustsible playbook examples/manual/local-smoke.yml \
  -i examples/manual/inventory-local.ini --forks 0
echo $?
```

预期：均返回非 0，并给出可诊断错误。

## 3. 远程 SSH 验收

在一次性 VM 上创建 inventory；不要提交真实地址、密码或密钥：

```ini
[manual_remote]
node1 ansible_host=192.0.2.10 ansible_user=tester ansible_connection=ssh ansible_ssh_private_key_file=/absolute/path/id_ed25519 rustsible_known_hosts_file=/absolute/path/known_hosts
node2 ansible_host=192.0.2.11 ansible_user=tester ansible_connection=ssh ansible_ssh_private_key_file=/absolute/path/id_ed25519 rustsible_known_hosts_file=/absolute/path/known_hosts

[manual_remote:vars]
ansible_ssh_timeout=5
ansible_command_timeout=10
```

`ssh-keyscan` 的结果本身不证明服务器身份；加入 known_hosts 前必须通过可信渠道核验
指纹。

### 3.1 Host-key 与认证顺序

1. 指向空 known_hosts 文件，执行 `command -a "id"`，预期在认证/执行前失败。
2. 加入已人工核验的正确 key，预期连接成功。
3. 替换为错误 key，预期明确拒绝。
4. 只有显式设置 `rustsible_host_key_checking=false` 时才允许关闭校验；完成后恢复。

### 3.2 SSH 命令、SFTP 与二进制保真

```bash
target/release/rustsible ad-hoc manual_remote -i /tmp/manual-remote.ini \
  -m command -a "id" --forks 2

target/release/rustsible ad-hoc manual_remote -i /tmp/manual-remote.ini \
  -m copy -a "src=/path/to/binary-fixture dest=/tmp/rustsible-binary mode=0600" --forks 2
```

在控制机和目标机分别运行 `sha256sum`，预期 hash 完全一致。重复 copy 预期
`changed=false`；加 `--check` 并换一个 dest，预期只报告变化且目标文件不存在。

### 3.3 超时、输出上限与并发

- 把 `ansible_command_timeout` 临时设为 `1`，运行 `shell -a "sleep 10"`，应约
  1 秒失败，不能永久挂住。
- 运行同时产生大量 stdout/stderr 的测试脚本，确认两路都被读取且不会死锁；超过
  16 MiB 合计上限时应被终止并明确报错。
- 在两台主机运行 `shell -a "sleep 2"`，比较 `--forks 1` 与 `--forks 2`；后者
  总耗时应明显更短，且不超过并发上限。
- 让 node1 命令失败、node2 成功，确认失败主机从后续 task 移除，而健康主机继续。

## 4. become 与状态模块验收

本节会修改系统，只能在一次性测试机执行。先使用 `--check --become`，确认预测
合理后再移除 `--check`。

### 4.1 提权与路径安全

```bash
target/release/rustsible ad-hoc manual_remote -i /tmp/manual-remote.ini \
  -m file -a "path=/opt/rustsible-manual state=directory mode=0750 owner=root group=root" \
  --become --check

target/release/rustsible ad-hoc manual_remote -i /tmp/manual-remote.ini \
  -m file -a "path=/opt/rustsible-manual state=directory mode=0750 owner=root group=root" \
  --become
```

验证 inventory 的 `ansible_become` / `ansible_become_user` 继承，以及 CLI 覆盖优先级。
密码不得出现在进程 argv、普通日志或 `-vvvv` 输出中。尝试删除空路径、`/`、`.`、
`..` 等危险路径，必须拒绝且不能产生任何删除。

### 4.2 package、service、user

为测试发行版选择一个安全、可恢复的软件包和服务：

```bash
target/release/rustsible ad-hoc manual_remote -i /tmp/manual-remote.ini \
  -m package -a "name=<safe-package> state=present" --become --check

target/release/rustsible ad-hoc manual_remote -i /tmp/manual-remote.ini \
  -m service -a "name=<safe-service> state=started" --become --check

target/release/rustsible ad-hoc manual_remote -i /tmp/manual-remote.ini \
  -m user -a "name=rustsible_manual_user state=present create_home=false shell=/bin/false" \
  --become --check
```

确认后实际执行两次：第一次按实际差异 changed，第二次应 `changed=false`。再验证：

- service 只给 `enabled` 时不应隐式启动服务；
- package `latest` 对未安装软件包会安装，而不是只做 upgrade；
- user 的 uid/gid/group/home/shell/comment/groups/password 差异可正确识别；
- user 密码记录通过 stdin 传递，用户名中的冒号/换行和未知参数被拒绝；
- `lineinfile backup=true` 只在真实内容变化时创建备份。

最后删除测试用户、目录，并把服务/软件包恢复到测试前快照。

## 5. 最终验收标准

以下条件全部满足才算通过：

- check mode 对所有测试均无落盘或系统变更；
- file/copy/template/lineinfile/package/service/user 第二次执行幂等；
- command/shell、rc、stdout/stderr/lines、loop results 和 register 可用；
- handler 仅在成功 changed 的对应 host 上执行；
- 未知或不支持语义明确失败，playbook/task 失败返回非 0；
- SSH 默认严格校验 host key，连接和命令 timeout 生效；
- 大量双向 I/O 不死锁，输出上限可终止失控进程；
- no_log、inventory-debug 和 verbose 日志中没有秘密值；
- shell 参数、路径、用户、软件包和服务名不能造成命令注入；
- 本地与远程结果一致，二进制 copy 保真；
- 自动质量门禁和 RustSec 审计均通过。

本地测试完成后清理：

```bash
target/release/rustsible playbook examples/manual/cleanup-local.yml \
  -i examples/manual/inventory-local.ini
```
