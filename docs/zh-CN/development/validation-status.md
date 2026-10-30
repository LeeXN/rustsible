[English / 简体中文](../../en/development/validation-status.md) · [Index / 目录](../index.md)

# 交付与验收记录

<a id="baseline"></a>

## 功能基线

22项 Linux/原生执行计划通过独立 PR 交付。原生集合/扩展在 PR23 合入（CI40978538184，8作业成功）；最终包验收在 PR24 合入（源码48c3f05a044f769691b7fc731ad6464a545b72d6，CI40985150671，9作业成功）。本次文档重组增加自身校验，不改写历史源码身份。

<a id="evidence"></a>

## 阅读证据

[PR23](https://github.com/LeeXN/rustsible/pull/23)、[PR24](https://github.com/LeeXN/rustsible/pull/24)、[最终CI](https://github.com/LeeXN/rustsible/actions/runs/40985150671)。产物含两种控制端包（各附双 worker）和原生集合；4组原生 glibc/musl 场景保存12份目标记录，跨架构另有记录。备份明确禁用输出换行裁剪，并在回滚后校验完整配置摘要。

<a id="limits"></a>

## 证据边界

使用当前[兼容矩阵](../reference/compatibility.md)，不笼统宣称完全等价 Ansible。DNF/Yum/Pacman/Zypper、Upstart 的真实验证仅 x86。cargo audit 记录既有 unsafe-libyaml 停止维护提示，不应添加通配忽略来刷绿门禁；新运行行为须补代表性目标状态断言。
