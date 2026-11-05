# 该选哪个 Native 后端

Native 隐藏 Java framework 以下的 VPN 特征，包括 libc、直接系统调用、netlink 和
`/proc/net`。安装**同一构建**的应用和**完整 `vpnhide-core.zip`**。Core 包含后端
payload，并自动选择兼容的实现。

**优先使用兼容的内核后端。** Zygisk 钩子运行在应用进程内；银行、反欺诈及其他受保护应用
可能检测到提供程序或注入代码并拒绝运行。Zygisk 是覆盖较少的后备方案，不是推荐的起点。
内核后端也不保证不可检测，更不会隐藏已安装的 Zygisk 提供程序或 root。
直接系统调用可以绕过 libc 钩子。

## 可选项

- **内置驱动：**Core 使用已经集成到内核中的兼容 VPN Hide 驱动。安装 Core 不会给普通
  内核添加该驱动。
- **Kmod：**Core 根据内核证据选择随包提供的兼容 `.ko`。见[内核模块](kmod-install.md)。
- **KPM：**随包提供的内核后端，需要可用且通过认证的 KernelPatch 运行时。
  见[KPM 要求](kpm-install.md)。
- **Zygisk：**随包提供的用户空间后端，需要兼容的 Zygisk 提供程序。直接系统调用可能绕过
  libc 钩子。见[Zygisk 要求](zygisk-install.md)。

Java/LSPosed 和 Native 覆盖不同路径。Apps 角色需要 Java/LSPosed；Ports 已包含在 Core 中。
见[检测向量](detection-vectors.md)。

## 只保留一个原生后端

Core 确保运行中的原生后端互斥。通用 Core ZIP 中的所有候选实现本来就应一起提供，
文件同时存在不构成冲突。旧的独立 VPN Hide 模块或自行加载的后端可能需要迁移。
按 Core 的恢复提示操作，在替代安装准备并验证完成前保留配置和可用安装。
不要手动加载内核 payload。

## 让仪表盘替你选

Core 负责选择，仪表盘显示选择、应用结果和保留的诊断证据。兼容性未知或覆盖不完整
会如实显示。存在后端文件不代表保护有效。见[模块未生效](module-not-active.md)。

## 高级内核选择

设置 → 原生后端（高级）保留保存/取消，并为下次设备启动记录选择。Auto 优先选择兼容的内置后端，再选择 Kmod；失败后不会自动尝试其他后端，也不会新启用 Zygisk。明确选择 Zygisk 时，必须确认注入与被检测风险警告。Core 会准备受管理的发现副本并检查完整策略及适用性；准备完成不代表当前保护。更新后的自愿内核迁移提示可以拒绝，以保留受管理的 Zygisk。安全切换准备期间，当前后端继续运行。Core 不会自动重启设备。
