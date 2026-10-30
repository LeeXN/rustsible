[English / 简体中文](README.md)

# Rustsible 示例

从仓库或离线包根目录运行。以下配置资源为两种语言共用，实际执行时必须使用新的私有目录。所有示例都可离线检查语法，但这不证明缺少前提资源的例子可直接部署。

| 场景 | 内容与前提 |
|---|---|
| [quickstart](quickstart/playbook.yml) | 本地文件、handler、二次幂等；必须提供 demo_root。 |
| [inventory](inventory/hosts) | INI 本地组选择，无远端地址和凭据。 |
| [variables](variables/playbook.yml) | 类型化循环与跨 play 变量；demo_root 须存在。 |
| [files](files/playbook.yml) | 模板和行修复，须提供 demo_root。 |
| [roles](roles/main.yml) | 静态导入、角色、handler、故意救援，须提供 demo_output。 |
| [selection](selection/playbook.yml) | deploy/diagnostics 标签，不写入目标。 |
| [rolling](rolling/playbook.yml) | 三台逻辑本地主机，demo_root 为新路径前缀；SSH 要另备已审查清单。 |
| [async](async/playbook.yml) | 原生作业启动/查询/清理，使用新的 demo_root。 |
| [assertions](assertions/playbook.yml) | 布尔条件和故意失败/救援，无业务文件。 |
| [vault](vault/playbook.yml) | 准备含 example_value 的加密 vault_variables，demo_root 须存在。 |
| [http](http/playbook.yml) | 须准备隔离的 health_host/health_port/health_url 服务，不自动安装服务器。 |
| [artifacts](artifacts/playbook.yml) | 须提供审查后的 source_repo/release_revision/archive_source、目标 Git 与 demo_root。 |
| [native-collection](native-collection/README.zh-CN.md) | 显式构建/安装原生示例集合，见其 README。 |

## 本地入门

```bash
demo_root=$(mktemp -d)
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root" --check --diff
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root"
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root"
rustsible playbook examples/quickstart/cleanup.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root"
```

其余状态模块在[模块参考](../docs/zh-CN/reference/modules.md)中提供完整 playbook。账号、软件包、服务例子仅在隔离主机执行。历史“全功能”教材已删除，未支持参数的最小回归样本在 tests/fixtures，不再作为公开教程。Rust 辅助程序属于开发/测试工具，不包含在离线用户示例中。
