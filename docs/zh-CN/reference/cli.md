[English / 简体中文](../../en/reference/cli.md) · [Index / 目录](../index.md)

# 命令行参考

由 Clap 生成。表中默认值为参数解析默认值，适用的配置和连接变量可以覆盖它们；有效优先级见配置与变量参考。命令和标识符不翻译。预演允许临时 worker 引导，禁止业务修改；语法和列表检查仅在控制端进行。

<a id="rustsible"></a>

## `rustsible`

Rust 编写的 Ansible 风格自动化工具

```text
Usage: rustsible [OPTIONS] <COMMAND>
```

| 参数 | 必填 | 默认值 | 含义 |
|---|---|---|---|
| `--vault-password-file` | false | — | 从不可执行的普通文件读取 Vault 密码 |
| `--vault-id` | false | — | Vault 密码来源：ID@FILE 或 ID@prompt |
| `--ask-vault-pass` | false | false | 在控制终端隐藏输入 Vault 密码 |
| `--config` | false | — | 明确指定 ansible.cfg 文件 |
| `--worker-dir` | false | — | 包含 worker 清单与程序的产物目录 |
| `-h, --help` | false | — | 显示帮助 |
| `-V, --version` | false | — | 显示版本 |

<a id="rustsible-build-info"></a>

## `rustsible build-info`

不加载配置，输出 JSON 构建身份

```text
Usage: rustsible build-info [OPTIONS]
```

| 参数 | 必填 | 默认值 | 含义 |
|---|---|---|---|
| `--config` | false | — | 明确指定 ansible.cfg 文件 |
| `--worker-dir` | false | — | 包含 worker 清单与程序的产物目录 |
| `-h, --help` | false | — | 显示帮助 |

<a id="rustsible-vault"></a>

## `rustsible vault`

加密、解密、查看或更换 Ansible Vault 密码

```text
Usage: rustsible vault [OPTIONS] <COMMAND>
```

| 参数 | 必填 | 默认值 | 含义 |
|---|---|---|---|
| `--vault-password-file` | false | — | 从不可执行的普通文件读取 Vault 密码 |
| `--vault-id` | false | — | Vault 密码来源：ID@FILE 或 ID@prompt |
| `--ask-vault-pass` | false | false | 在控制终端隐藏输入 Vault 密码 |
| `--config` | false | — | 明确指定 ansible.cfg 文件 |
| `--worker-dir` | false | — | 包含 worker 清单与程序的产物目录 |
| `-h, --help` | false | — | 显示帮助 |

<a id="rustsible-vault-encrypt"></a>

## `rustsible vault encrypt`



```text
Usage: rustsible vault encrypt [OPTIONS] <FILE>
```

| 参数 | 必填 | 默认值 | 含义 |
|---|---|---|---|
| `--vault-password-file` | false | — | 从不可执行的普通文件读取 Vault 密码 |
| `--vault-id` | false | — | Vault 密码来源：ID@FILE 或 ID@prompt |
| `--ask-vault-pass` | false | false | 在控制终端隐藏输入 Vault 密码 |
| `input` | true | — | Vault 输入文件，- 表示标准输入 |
| `--output` | false | — | 输出文件，- 表示标准输出，默认替换输入文件 |
| `--encrypt-vault-id` | false | — | 选择加密标签及匹配的密码来源 |
| `--config` | false | — | 明确指定 ansible.cfg 文件 |
| `--worker-dir` | false | — | 包含 worker 清单与程序的产物目录 |
| `-h, --help` | false | — | 显示帮助 |

<a id="rustsible-vault-decrypt"></a>

## `rustsible vault decrypt`



```text
Usage: rustsible vault decrypt [OPTIONS] <FILE>
```

| 参数 | 必填 | 默认值 | 含义 |
|---|---|---|---|
| `--vault-password-file` | false | — | 从不可执行的普通文件读取 Vault 密码 |
| `--vault-id` | false | — | Vault 密码来源：ID@FILE 或 ID@prompt |
| `--ask-vault-pass` | false | false | 在控制终端隐藏输入 Vault 密码 |
| `input` | true | — | Vault 输入文件，- 表示标准输入 |
| `--output` | false | — | 输出文件，- 表示标准输出，默认替换输入文件 |
| `--config` | false | — | 明确指定 ansible.cfg 文件 |
| `--worker-dir` | false | — | 包含 worker 清单与程序的产物目录 |
| `-h, --help` | false | — | 显示帮助 |

<a id="rustsible-vault-view"></a>

## `rustsible vault view`



```text
Usage: rustsible vault view [OPTIONS] <FILE>
```

| 参数 | 必填 | 默认值 | 含义 |
|---|---|---|---|
| `--vault-password-file` | false | — | 从不可执行的普通文件读取 Vault 密码 |
| `--vault-id` | false | — | Vault 密码来源：ID@FILE 或 ID@prompt |
| `--ask-vault-pass` | false | false | 在控制终端隐藏输入 Vault 密码 |
| `input` | true | — | Vault 输入文件，- 表示标准输入 |
| `--config` | false | — | 明确指定 ansible.cfg 文件 |
| `--worker-dir` | false | — | 包含 worker 清单与程序的产物目录 |
| `-h, --help` | false | — | 显示帮助 |

<a id="rustsible-vault-rekey"></a>

## `rustsible vault rekey`



```text
Usage: rustsible vault rekey [OPTIONS] <FILE>
```

| 参数 | 必填 | 默认值 | 含义 |
|---|---|---|---|
| `--vault-password-file` | false | — | 从不可执行的普通文件读取 Vault 密码 |
| `--vault-id` | false | — | Vault 密码来源：ID@FILE 或 ID@prompt |
| `--ask-vault-pass` | false | false | 在控制终端隐藏输入 Vault 密码 |
| `input` | true | — | Vault 输入文件，- 表示标准输入 |
| `--output` | false | — | 输出文件，- 表示标准输出，默认替换输入文件 |
| `--encrypt-vault-id` | false | — | 选择加密标签及匹配的密码来源 |
| `--new-vault-password-file` | false | — | rekey 使用的新密码文件，可重复提供 |
| `--new-vault-id` | false | — | rekey 新密码来源 ID@FILE 或 ID@prompt，可重复提供 |
| `--config` | false | — | 明确指定 ansible.cfg 文件 |
| `--worker-dir` | false | — | 包含 worker 清单与程序的产物目录 |
| `-h, --help` | false | — | 显示帮助 |

<a id="rustsible-collection"></a>

## `rustsible collection`

打包、校验或安装原生集合

```text
Usage: rustsible collection [OPTIONS] <COMMAND>
```

| 参数 | 必填 | 默认值 | 含义 |
|---|---|---|---|
| `--config` | false | — | 明确指定 ansible.cfg 文件 |
| `--worker-dir` | false | — | 包含 worker 清单与程序的产物目录 |
| `-h, --help` | false | — | 显示帮助 |

<a id="rustsible-collection-pack"></a>

## `rustsible collection pack`



```text
Usage: rustsible collection pack [OPTIONS] --output <output> <input>
```

| 参数 | 必填 | 默认值 | 含义 |
|---|---|---|---|
| `input` | true | — | 集合源码目录 |
| `--output` | true | — | 新集合归档路径，不覆盖已有文件 |
| `--config` | false | — | 明确指定 ansible.cfg 文件 |
| `--worker-dir` | false | — | 包含 worker 清单与程序的产物目录 |
| `-h, --help` | false | — | 显示帮助 |

<a id="rustsible-collection-verify"></a>

## `rustsible collection verify`



```text
Usage: rustsible collection verify [OPTIONS] <input>
```

| 参数 | 必填 | 默认值 | 含义 |
|---|---|---|---|
| `input` | true | — | 待校验或安装的集合归档 |
| `--config` | false | — | 明确指定 ansible.cfg 文件 |
| `--worker-dir` | false | — | 包含 worker 清单与程序的产物目录 |
| `-h, --help` | false | — | 显示帮助 |

<a id="rustsible-collection-install"></a>

## `rustsible collection install`



```text
Usage: rustsible collection install [OPTIONS] <input>
```

| 参数 | 必填 | 默认值 | 含义 |
|---|---|---|---|
| `input` | true | — | 待校验或安装的集合归档 |
| `--path` | false | — | 集合安装根目录，自定义根目录仍须加入搜索路径 |
| `--config` | false | — | 明确指定 ansible.cfg 文件 |
| `--worker-dir` | false | — | 包含 worker 清单与程序的产物目录 |
| `-h, --help` | false | — | 显示帮助 |

<a id="rustsible-playbook"></a>

## `rustsible playbook`

运行 Rustsible playbook

```text
Usage: rustsible playbook [OPTIONS] <playbook>
```

| 参数 | 必填 | 默认值 | 含义 |
|---|---|---|---|
| `--collections-path` | false | — | 显式集合根目录，同时授权已声明的原生程序 |
| `--vault-password-file` | false | — | 从不可执行的普通文件读取 Vault 密码 |
| `--vault-id` | false | — | Vault 密码来源：ID@FILE 或 ID@prompt |
| `--ask-vault-pass` | false | false | 在控制终端隐藏输入 Vault 密码 |
| `-e, --extra-vars` | false | — | 额外变量：name=value、YAML/JSON 映射或 @file |
| `-k, --ask-pass` | false | false | 在控制终端隐藏输入 SSH 密码 |
| `-K, --ask-become-pass` | false | false | 在控制终端隐藏输入提权密码 |
| `--private-key` | false | — | SSH 私钥文件 |
| `--diff` | false | false | 显示支持模块的有界结构化差异 |
| `--tags` | false | — | 运行匹配逗号分隔 tags 的任务 |
| `--skip-tags` | false | — | 跳过匹配逗号分隔 tags 的任务 |
| `--syntax-check` | false | false | 不连接目标，检查静态 playbook 结构 |
| `--list-hosts` | false | false | 不连接目标，输出选中主机 JSON |
| `--list-tasks` | false | false | 不连接目标，输出选中静态任务 JSON |
| `playbook` | true | — | 运行的 playbook 文件 |
| `-i, --inventory` | false | — | 指定 inventory 路径，默认 inventory |
| `-v` | false | 0 | 增加详细程度，最多 -vvvv |
| `-l, --limit` | false | — | 限制到指定主机或组 |
| `--check` | false | false | 预演业务变化，允许临时 worker 引导 |
| `-f, --forks` | false | 5 | 并行处理主机数量上限 |
| `--config` | false | — | 明确指定 ansible.cfg 文件 |
| `--worker-dir` | false | — | 包含 worker 清单与程序的产物目录 |
| `-h, --help` | false | — | 显示帮助 |

<a id="rustsible-ad-hoc"></a>

## `rustsible ad-hoc`

在目标主机运行临时模块命令

```text
Usage: rustsible ad-hoc [OPTIONS] --module <MODULE> <pattern>
```

| 参数 | 必填 | 默认值 | 含义 |
|---|---|---|---|
| `--collections-path` | false | — | 显式集合根目录，同时授权已声明的原生程序 |
| `--vault-password-file` | false | — | 从不可执行的普通文件读取 Vault 密码 |
| `--vault-id` | false | — | Vault 密码来源：ID@FILE 或 ID@prompt |
| `--ask-vault-pass` | false | false | 在控制终端隐藏输入 Vault 密码 |
| `-e, --extra-vars` | false | — | 额外变量：name=value、YAML/JSON 映射或 @file |
| `-k, --ask-pass` | false | false | 在控制终端隐藏输入 SSH 密码 |
| `-K, --ask-become-pass` | false | false | 在控制终端隐藏输入提权密码 |
| `--private-key` | false | — | SSH 私钥文件 |
| `--diff` | false | false | 显示支持模块的有界结构化差异 |
| `pattern` | true | — | 目标主机选择模式 |
| `-m, --module` | true | — | 执行的模块名称 |
| `-a, --args` | false | — | 模块参数 |
| `-i, --inventory` | false | — | 指定 inventory 路径，默认 inventory |
| `-v` | false | 0 | 增加详细程度，最多 -vvvv |
| `-b, --become` | false | false | 使用提权执行操作 |
| `--become-user` | false | — | 以指定用户执行，默认 root |
| `-B, --background` | false | 0 | 后台任务最长运行时间，零表示同步执行 |
| `-P, --poll` | false | 15 | 后台任务轮询间隔，零表示立即返回 job ID |
| `--check` | false | false | 预测业务变化而不应用 |
| `-f, --forks` | false | 5 | 并行处理主机数量上限 |
| `--config` | false | — | 明确指定 ansible.cfg 文件 |
| `--worker-dir` | false | — | 包含 worker 清单与程序的产物目录 |
| `-h, --help` | false | — | 显示帮助 |

<a id="rustsible-inventory-debug"></a>

## `rustsible inventory-debug`

检查并展示主机清单信息

```text
Usage: rustsible inventory-debug [OPTIONS]
```

| 参数 | 必填 | 默认值 | 含义 |
|---|---|---|---|
| `--collections-path` | false | — | 显式集合根目录，同时授权已声明的原生程序 |
| `--vault-password-file` | false | — | 从不可执行的普通文件读取 Vault 密码 |
| `--vault-id` | false | — | Vault 密码来源：ID@FILE 或 ID@prompt |
| `--ask-vault-pass` | false | false | 在控制终端隐藏输入 Vault 密码 |
| `-i, --inventory` | false | — | 指定 inventory 路径，默认 inventory |
| `-v` | false | 0 | 增加详细程度，最多 -vvvv |
| `--config` | false | — | 明确指定 ansible.cfg 文件 |
| `--worker-dir` | false | — | 包含 worker 清单与程序的产物目录 |
| `-h, --help` | false | — | 显示帮助 |
