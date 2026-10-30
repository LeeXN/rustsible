[English / 简体中文](../../en/tutorials/local.md) · [Index / 目录](../index.md)

# 首次本地部署

<a id="files"></a>

## 准备文件

从源码或解包根目录运行。示例 inventory 明确选择 local，playbook.yml 创建 app 目录和配置，cleanup.yml 仅删除该 app 目录。demo_root 必须传入，并应是新建的私有目录；不涉及 sudo、系统账号或软件包操作。

<a id="run"></a>

## 检查、预演、执行与重跑

```bash
demo_root=$(mktemp -d)
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root" --syntax-check
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root" --check --diff
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root"
cat "$demo_root/app/config"
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root"
rustsible playbook examples/quickstart/cleanup.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root"
```

<a id="observe"></a>

## 验证目标结果

语法检查输出 JSON，不连接目标。预演报告文件变化，但不创建 app/。首次执行写入 application=rustsible 和 enabled=true，权限为 0600；被通知的 handler 写入 applied。第二次执行应 changed=0。清理仅删除 app/，调用者持有的空 demo_root 保留。check 可以临时上传并删除 worker，不能理解为完全没有临时基础设施写入。

<a id="next"></a>

## 理解执行过程

接着阅读[变量](../guides/variables.md)、[文件模块](../reference/modules/copy.md)与[角色部署](deployment.md)。中英文使用同一示例和相同可执行代码。
