# 安装 Zygisk 模块

Zygisk 是用户空间的 Native 后端：它在**每个目标应用进程内部**挂钩 libc。当没有
内核后端适配你的设备时才用它。它比内核后端更弱——请先阅读
[该选哪个 Native 后端](choosing-native.md)。

## 需要知道的

- **原始 `svc` 系统调用能绕过它。**直接发起系统调用、跳过 libc 的应用不会被过滤。
  内核后端（kmod / KPM）没有这个缺口。
- **银行与支付类应用可能察觉它。**挂钩位于应用自身进程内，部分反篡改 SDK 会注意
  到。对这类应用，请让 **Native 保持关闭**，改依赖 Java 层——为它们开启 Native
  可能让它们*更*可疑，而非相反。仪表盘会显示同样的警告。

如果你的内核支持 kmod 或 KPM，请优先选它们；仪表盘会提示。

## 要求

- 一个 **Zygisk 实现**：Magisk 自带的 Zygisk，或 Magisk / KernelSU 上的
  ZygiskNext / NeoZygisk。
- 为 Java 层启用 **LSPosed / Vector**（每个后端都需要的同一个搭档）。

## 步骤

1. 下载 `vpnhide-zygisk.zip`（仪表盘会指明文件）。
2. 在 root 管理器中打开 **Modules → Install from storage** 并选择该 ZIP。
3. **重启。**
4. 在 VPN Hide 应用里选择目标应用。
5. 打开**仪表盘**；Native 后端应显示 **Active**。

## 更改如何生效

Zygisk 挂钩在应用进程启动时安装，因此配置更改会在下次全新启动时生效。对目标应用
执行 **Force-stop**（或重启）再重新打开即可应用——见
[设置隐藏](configure-hiding.md)。
