[English / 简体中文](../../en/reference/facts.md) · [Index / 目录](../index.md)

# Facts 与目标信息

<a id="collection"></a>

## 收集与过滤

默认在每个选中目标收集 facts。setup 接受 gather_subset（默认 all）、filter（顶层名称 Shell 模式）、gather_timeout（1–604800秒，默认10）；显式 setup 合并同一主机状态。禁用 facts 不会伪造控制端时间或目标信息。

<a id="subsets"></a>

## 支持的子集

platform：architecture/system/kernel/kernel_version/machine/hostname/nodename/fqdn/domain。distribution：发行版名称/版本/主版本/release/os_family。date_time：日期时间组件、epoch、ISO8601。network：interfaces/all_ipv4_addresses/all_ipv6_addresses/default_ipv4。min 含 platform/distribution/date_time，all 加 network；可选择支持的单项。!all 保留 min，!all,!min 从空集开始。

<a id="access"></a>

## 访问与未提供内容

setup 结果键带 ansible_ 前缀，ansible_facts 内去掉前缀；如 ansible_architecture 与 ansible_facts.architecture。set_fact 使用自定义名称。没有持久缓存、Python 收集器、任意可执行 local facts 或完整 Ansible facts 集；委派归属由 delegate_facts 控制。
