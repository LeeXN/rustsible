# Rustsible

[English / 简体中文](README.md) · [MIT](LICENSE) · [CI](https://github.com/LeeXN/rustsible/actions/workflows/rust.yml)

Rustsible 是 Rust 编写的 Linux 自动化工具：控制端组织任务，通过本地或 SSH 临时投放的原生 worker 操作目标，无常驻 Agent，也没有内置 Python/curl 运行依赖。支持 Linux x86_64、aarch64 及声明范围内的 glibc/musl 环境。

兼容性以固定 Ansible 2.21 的明确子集为准，不宣称完整生态兼容。25 个内置模块、角色与失败处理、滚动发布、异步、Vault、受控原生集合的参数与限制均有独立参考。系统工具可能有自身依赖，例如 DNF/Yum 的操作系统 Python。

## 开始使用

- [安装、构建与离线包校验](docs/zh-CN/tutorials/install.md)
- [首次本地部署与幂等检查](docs/zh-CN/tutorials/local.md)
- [首次 SSH 部署与身份验证](docs/zh-CN/tutorials/ssh.md)
- [角色、handler 与失败救援](docs/zh-CN/tutorials/deployment.md)
- [完整使用与开发手册](docs/zh-CN/index.md)
- [25 个模块与能力索引](docs/zh-CN/reference/modules.md)
- [CLI 参考](docs/zh-CN/reference/cli.md)
- [支持矩阵与安全差异](docs/zh-CN/reference/compatibility.md)
- [原生扩展开发](docs/zh-CN/development/native-extensions.md)
- [贡献与验证流程](docs/zh-CN/development/contributing.md)

## 最小本地示例

先按安装教程准备匹配的控制端与 workers，随后从源码或离线包根目录执行；示例只操作新目录。

```bash
demo_root=$(mktemp -d)
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root" --check --diff
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root"
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root"
```

第二次执行应 changed=0。清理与预期文件内容见教程。全局 --check 不允许 task check_mode:false 修改业务状态，但可以上传并清理临时 worker。no_log 遮蔽展示而非清空 register；修改结果 uncertain 时不自动重放。

离线包包含双 worker、双语手册、示例、RELEASE.json 与递归 SHA256SUMS。原生集合只在显式安装或配置后获得执行授权；Python 插件不会回退运行。实际平台/后端验收边界与历史证据见手册，不把交叉编译当原生执行。
