# 原生 collections 与扩展

Rustsible 读取 collection 中已支持的 roles、playbooks，并运行显式授权的 Linux ELF 扩展。它不运行 Python 模块、Python 插件、Rust 动态库，也不自动安装 Galaxy 依赖。扩展是用户信任的本地程序，不是沙箱；清单和 SHA-256 用于完整性检查，不是作者签名。

## 内容与搜索

目录布局为 `ROOT/ansible_collections/namespace/collection`。可以使用 `namespace.collection.role` 和 `namespace.collection.playbook`；play 的 `collections: [namespace.collection]` 负责模块短名称搜索。集合角色先搜索自己的集合，普通角色不继承 play 的模块搜索列表。内置 FQCN 保留，不能被扩展覆盖。

搜索顺序是重复的 `--collections-path ROOT`、项目 `collections/`、`~/.rustsible/collections`。没有 CLI 路径时，使用 `[defaults] collections_path`，环境 `ANSIBLE_COLLECTIONS_PATH` 覆盖配置文件。显式配置的路径授权其清单声明的程序；项目目录仅提供声明式内容。默认安装目录还要求集合内容之外的 `~/.rustsible/native-trust.json` 安装记录；复制仓库自带的收据不能获得执行权限。

```sh
rustsible collection pack SOURCE --output example.tar.gz
rustsible collection verify example.tar.gz
rustsible collection install example.tar.gz
rustsible collection install example.tar.gz --path /opt/rustsible/collections
rustsible playbook example.native.inspect -i inventory.yml
rustsible ad-hoc all -i inventory.yml --collections-path /opt/rustsible/collections \
  -m example.native.probe -a 'path=/etc/passwd'
```

自定义安装目录仍需通过 CLI 或配置显式加入搜索路径。安装不会执行包内容。包包含 `FILES.json`、`rustsible-collection.json` 及全部文件摘要。解包先完整检查路径、文件类型、清单、文件集合和 ELF；拒绝链接、设备、越界路径、未声明文件及 Python 插件。更新通过同目录原子目录替换完成；已有文件或空目录包含未托管内容时拒绝覆盖。

可直接构建的完整例子见 [examples/native-collection](examples/native-collection/README.md)。例子包含模块、inventory、lookup、filter、角色和 playbook；源码在 `crates/native-example`。发布例子使用静态程序，目标不需要 Rust 工具链、Python 或 curl。

## 可执行接口 v1

协议定义与 SDK 在 `rustsible-protocol::extension`，示例使用 `serve(catalog, handler)`。每次调用通过 stdin/stdout 的大端 u32 长度 JSON 帧通信：

1. 控制端或 worker 发送只含组件身份的 `Probe`。
2. 程序返回 `Hello`；API 版本、wire 版本、名称、种类、能力和参数定义必须与清单一致。
3. 握手通过后才发送参数 `Call`，返回统一的 `Reply`/`ModuleResult`。

stdout 专用于协议；stderr 最多 64 KiB，作为受限诊断处理，不直接反射提供器的敏感输出。请求与扩展结果上限均为 16 MiB，ELF 上限 256 MiB；集合最多 100000 个文件、展开总量 8 GiB。模块参数按清单声明的原生类型严格校验，未知参数报错。

清单包含 `format: 1`、`namespace/name/version`、`extensions`。每个组件有 `definition` 和按 `x86_64/aarch64` 索引的 `artifacts`（相对 `path`、`sha256`）。`definition` 包含 FQCN、`kind`、`capabilities` 和 `parameters`；能力包括 `check/diff/async/mutates`。程序必须是当前身份或 root 所有，文件及集合内父目录不得允许组或其他用户写入。每次执行重新验证实际目标架构、ELF 和摘要。控制端提供器必须有控制端架构产物；目标模块按真实目标架构选择，不使用可覆写 facts 猜架构。

| 类型 | 执行位置 | 参数及结果 |
|---|---|---|
| module | 目标身份的原生 worker | 声明的模块参数；统一结果，支持声明的 check/diff/async |
| inventory | 控制端 | YAML 配置中的 `plugin: FQCN` 选择提供器，其余键作为参数；结果 `values.inventory` 使用 YAML inventory 对应的 JSON 结构 |
| lookup | 控制端 | `terms` 列表、`keywords` 映射；结果 `values.value` |
| filter | 控制端 | `value`、`arguments` 列表、`keywords` 映射；结果 `values.value` |

inventory/lookup/filter 必须声明只读、支持 check，且不支持 diff/async。模板用 `lookup('namespace.collection.name', ...)` 和 FQCN filter。外部数据保留原生类型，字符串中的模板分隔符不会再次执行。传统 Ansible Python 动态 inventory JSON、Python lookup/filter 的调用约定不属于这个原生接口。

执行程序具有独立构建身份，但必须匹配 API/wire 版本和清单定义。崩溃、超时或协议中断若发生在修改请求传递之后，结果为“不确定”，不自动重试。监督进程回收脱离会话的后代。异步模块复制到执行身份的私有 job 目录，后续无 collection 配置的控制端仍可通过 `async_status` 查询、清理。全局 check 不启动异步任务。

## 编译期 Rust 接口

定制控制端通过 `Registry::new(...).with_compiled(Components { ... })` 注册，在分享 registry 前完成注册，再传入 `ExecutionOptions.extensions` 或 `AdHocOptions.extensions`。不扫描仓库寻找可执行插件。实际接入例子和检查见 [tests/compiled_extensions.rs](tests/compiled_extensions.rs)。

| 接口 | 首批边界 |
|---|---|
| action | 带同一参数定义与能力校验的控制端 Rust 函数；接收 Call、变量；不支持 async 或 task timeout |
| callback | 每次实际模块完成后的主机、模块、outcome、changed、check；不提供参数、消息或结果正文；异常回调不会让已执行任务自动重放 |
| connection | 按 `ansible_connection` 选择编译期工厂；提供命令通道及同一受限 worker Session；仍执行平台、UID、协议及产物验证 |
| become | 按 `ansible_become_method` 选择编译期函数；返回 bootstrap 命令与独立 stdin 认证前缀，后续仍验证执行身份 |
| strategy | 指定 linear/free 屏障引擎和主机排列函数；排列必须是完整无重复的已选主机集合，serial/throttle/forks/失败阈值仍由原调度器执行 |

这些是可信 Rust 源码接口，不承诺稳定 Rust ABI，也不宣称支持任意 Ansible action、callback 或 strategy 插件。编译期函数在控制端进程中执行，需要自行遵守 check 和期限；目标模块优先使用可监督、可限时的原生可执行接口。嵌入原生 lookup/filter 的自定义控制端，必须在入口调用 `rustsible_native::rpc::handle_runner_mode()`，与内置控制端相同。
