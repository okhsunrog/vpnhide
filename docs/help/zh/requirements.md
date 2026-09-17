# 要求与兼容性

VPN Hide 需要 **root**。按目标应用的检测方式启用所需层；Java + Native 是常用起点，
但不是必须同时启用所有层。

| 组件 | 要求 |
|---|---|
| 应用 | Android 9+，arm64 或 armv7，并授予 VPN Hide root 权限 |
| Java 和 Apps 角色 | LSPosed / LSPosed-Next / Vector，并为 VPN Hide 启用 System Framework 作用域 |
| Native — kmod | 兼容的 arm64 GKI 内核及所需探针支持；按仪表盘推荐选择 |
| Native — KPM | 支持的 arm64 内核家族与 KernelPatch 运行时，如 APatch 或 KPatch-Next |
| Native — Zygisk | 可用的 Zygisk 实现，如 Magisk Zygisk 或 ZygiskNext/NeoZygisk |
| Ports | 通过受支持的 root 管理器安装 Ports 模块 |

内核后端仅支持 arm64。没有兼容内核后端时，[Zygisk](zygisk-install.md)是覆盖较少的
后备选择。KernelSU 本身不提供此后端所需的 Zygisk 实现。

## 安装什么

从[首次安装](first-install.md)和[选择一个 Native 后端](choosing-native.md)开始。
内核钩子不向目标进程注入 libc 钩子，但不能承诺躲过所有反篡改或 root 检测。
VPN Hide 不隐藏 root，也不提供 Play Integrity 认证。

Built-in 是**自行编译内核**的高级集成方案，不是受支持的预编译内核下载选项。
见[选择 Native](choosing-native.md)中的说明。
