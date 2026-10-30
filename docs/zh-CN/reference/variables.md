[English / 简体中文](../../en/reference/variables.md) · [Index / 目录](../index.md)

# 变量来源与优先级

<a id="layers"></a>

## 普通变量分层

从低到高：配置默认值；role defaults；inventory 组再主机；facts；play vars；vars_files；role vars；task vars；include_vars；set_fact/register；include/role 参数；extra vars。role defaults 低于 inventory，role vars 属于对应角色作用域；别名在每层归一化。任务连接关键词默认值低于连接变量，循环绑定和当前尝试的 register 是临时执行覆盖层。

<a id="sources"></a>

## 加载与生命周期

group_vars/host_vars 依次在清单与 playbook 旁加载，支持无扩展名/YAML/JSON 文件及排序目录。vars_files 支持有序候选和来源相对路径，依赖 facts 的路径等 facts 可用后再解析；include_vars 仅选中时执行。facts/register 跨 play 保留，play/task 局部变量不泄漏；extra vars 支持重复 key=value、JSON/YAML 对象或 @file。

<a id="magic"></a>

## 运行时名称

hostvars、groups、group_names、inventory_hostname、inventory_file、inventory_dir、playbook_dir、ansible_config_file、ansible_play_hosts_all、ansible_play_batch 描述当前作用域。角色暴露 role_path、短 role_name、完整 ansible_role_name、ansible_collection_name。ansible_check_mode/ansible_diff_mode 反映调用开关，普通变量不能伪造；任务 check_mode:true 不会改写全局开关。

<a id="types"></a>

## 模板与数据边界

MiniJinja 单表达式保留布尔、列表、数字、字符串类型，混合文本保持字符串。定义解析引用并检测循环，命令及原生提供器输出保持普通数据；不猜测字符串 YAML 类型、不重复渲染。when/until/assert 要求布尔值，不能用任意非空字符串。default(omit) 可移除参数，password_hash 应提供明确稳定的 salt。
