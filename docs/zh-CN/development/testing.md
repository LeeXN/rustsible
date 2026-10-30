[English / 简体中文](../../en/development/testing.md) · [Index / 目录](../index.md)

# 测试与验收

<a id="layers"></a>

## 各层证明什么

单元测试覆盖解析、校验、状态、协议；本地集成检查真实文件、作用域、循环、重试、隐私。SSH 夹具验证严格信任、密码/密钥认证、三种身份、断连、超时、清理；服务/软件包夹具验证真实系统工具，不用模拟成功代替。行覆盖率门槛是回归门禁，不是语义证明。

<a id="local"></a>

## 运行本地检查

集成/覆盖率测试前构建 workspace，确保独立 worker 与 SDK 程序存在。测试使用私有临时资源，普通 Cargo 测试不要求 Python。执行[贡献检查](contributing.md#checks)；文档测试对精选本地示例运行两次并比较 check 快照。

<a id="ssh"></a>

## 一次性 SSH 与系统夹具

```bash
bash tests/ssh/run.sh CONTROLLER WORKERS
bash tests/ssh/run.sh CONTROLLER WORKERS reuse musl
bash tests/ssh/run.sh CONTROLLER WORKERS strategy glibc
bash tests/ssh/run.sh CONTROLLER WORKERS async glibc
bash tests/ssh/run.sh CONTROLLER WORKERS network glibc HTTP_FIXTURE
bash tests/managers/run.sh CONTROLLER WORKERS dnf
bash tests/managers/upstart.sh CONTROLLER WORKERS HTTP_FIXTURE
```

<a id="reference"></a>

## 对照与证据

将大写路径替换为匹配程序。http_fixture 来自 worker example，最小镜像使用静态版本；prompt_fixture 为控制端 example，供 operator/Vault 模式使用。固定对照命令为 bash tests/reference/run.sh CONTROLLER HTTP_FIXTURE，单独用 uv、ansible-core==2.21.0，不构成产品依赖。ARM、x86 原生作业各自证明实际架构，QEMU 控制端跨架构场景另行标注。检查字节/权限/属主/摘要与残留，不只看 rc/changed。
