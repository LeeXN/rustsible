[English / 简体中文](../../en/tutorials/deployment.md) · [Index / 目录](../index.md)

# 角色、通知与救援部署

<a id="layout"></a>

## 文件与职责

使用 examples/roles：main.yml 静态导入 deploy.yml；roles/web/tasks 渲染角色模板，handlers 记录配置变化；tasks/health.yml 检查文件，block/rescue/always 展示救援。只有 inventory 和入口 playbook 是基本必需文件；ansible.cfg、defaults、vars、handlers 按需添加。来源文件的相对路径不依赖启动目录。

<a id="execute"></a>

## 运行示例

```bash
demo_output=$(mktemp -d)
rustsible playbook examples/roles/main.yml -i examples/quickstart/inventory.yml -e "demo_output=$demo_output" --check
rustsible playbook examples/roles/main.yml -i examples/quickstart/inventory.yml -e "demo_output=$demo_output"
```

<a id="production"></a>

## 从示例到滚动发布

示例仅在非 check 下故意执行 /bin/false 展示救援；它不用于证明任意 shell 命令幂等。三机流程使用 examples/rolling，并传入新的 demo_root；serial [1,2] 先运行一台再两台，run_once 按批次选主，委派回执仍归属逻辑主机。详见[调度](../guides/scheduling.md)。下载→Vault 配置→handler→健康检查→逐字节回滚的完整场景由隔离发布验收维护。
