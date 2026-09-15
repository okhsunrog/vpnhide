# 要求与兼容性

VPN Hide 是一组 root 模块加上这个应用。你需要 root，并至少启用 Java 层；原生层需要
兼容的内核或 KernelPatch 运行时。

## 你需要什么

| 组件 | 要求 |
|---|---|
| 应用（选择器） | Android 9+，arm64 或 armv7 |
| Java 层 | LSPosed、LSPosed-Next 或 Vector |
| 原生 — kmod | 带 `CONFIG_KPROBES` 的 GKI 内核（Android 12+ 标配） |
| 原生 — KPM | APatch 或 KPatch-Next-Module（KernelPatch 运行时） |
| 原生 — Zygisk | Zygisk（Magisk / KernelSU）或 ZygiskNext |
| Ports（可选） | 任意 root 管理器 |

只安装**一个**原生后端、Java 层，以及可选的 Ports。应用本身也能在 32 位（armv7）
设备上运行，但内核后端仅支持 arm64。

## 如何选择原生后端

只要设备支持，优先用内核后端——`kmod` 或内置的内核补丁：它们在内核空间挂钩，银行和
支付类应用看不到。`Zygisk` 在任何支持 Zygisk 的地方都能用，但部分此类应用能察觉到它
在进程内的钩子。仪表盘会针对你的设备推荐一个后端。
