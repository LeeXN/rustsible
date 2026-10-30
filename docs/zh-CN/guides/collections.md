[English / 简体中文](../../en/guides/collections.md) · [Index / 目录](../index.md)

# 安装与使用原生集合

<a id="purpose"></a>

## 开始之前

项目 collections 中可以发现声明式角色/playbook，但不会自动授权仓库程序执行。原生执行需要显式安装，或通过 --collections-path/配置授权；默认安装还在包内容之外保存授权记录。

<a id="usage"></a>

## 用法

```bash
rustsible collection verify example.native-0.1.0.tar.gz
rustsible collection install example.native-0.1.0.tar.gz
rustsible playbook example.native.inspect -i inventory.yml
```

<a id="behavior"></a>

## 行为与边界

使用 namespace.collection.component 完整名称。先搜索显式根目录，再搜索项目 collections，最后搜索用户安装集合。collections_path/ANSIBLE_COLLECTIONS_PATH 指定配置根目录；自定义 --path 安装也须配置搜索路径。安装前检查 ELF、架构、摘要和全部文件，再原子安装；拒绝 Python 插件、链接、不安全属主和篡改。

<a id="reference"></a>

## 相关参考

参见[模块索引](../reference/modules.md)、[配置](../reference/configuration.md)、[关键词](../reference/keywords.md)和[结果](../reference/results.md)。
