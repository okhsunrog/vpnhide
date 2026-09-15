# 安装 KPM 后端

KPM 是面向可加载 `.ko` **无法**支持的内核的内核级 Native 后端——老旧或非 GKI
内核（如 4.9、4.14）、无源码的厂商内核、拒绝未签名模块的内核。它像 `.ko` 一样在
内核级别隐藏，因此对反篡改不可见。它是**单一通用模块**——无需匹配 GKI 版本。请先
阅读[该选哪个 Native 后端](choosing-native.md)。

## 它需要 KernelPatch 运行时

KPM 使用内核 inline 挂钩，这要求内核中已存在 KernelPatch 运行时。按你的 root
选择路径：

- **APatch / FolkPatch**——已内置 KernelPatch。直接安装模块 ZIP 并保存
  **SuperKey**（见下）。
- **KernelSU-Next**——先刷入 **KPatch-Next**。
- **Magisk 或原版 KernelSU**——安装独立的 **KPatch-Next-Module** 并从它的界面为
  内核打补丁。它在 Magisk 和 KernelSU 上都能用，因此无需改用 APatch。

若运行时不存在，仪表盘会提示*“内核未打入 KernelPatch 运行时”*。

## 步骤

1. 确认已存在 KernelPatch 运行时（见上）。
2. 下载 `vpnhide-kpm.zip`（仪表盘会指明文件）。
3. 通过 root 管理器的 **Modules** 界面，把**整个 ZIP** 作为 APM/Magisk 兼容模块
   安装。**不要**单独解压或加载内部的 `vpnhide.kpm` 文件——单独的它没有激活器、
   开机脚本和配置下发，隐藏不会生效。
4. 在 APatch / FolkPatch 上，把你的 **SuperKey** 保存到 **Settings → Security**
   （与你在 APatch/FolkPatch 里设置的相同）。
5. **重启**，然后打开**仪表盘**——Native 后端应显示 **Active**。

## 受支持的内核

KPM 在加载时校验你的内核家族。受支持的家族为 4.9、4.14、4.19、5.4、5.10、5.15、
6.1、6.6、6.12。其他家族会被拒绝而非猜测。若仪表盘显示*“内核不受支持……没有已
验证的偏移表”*，请改用 [Zygisk 后端](zygisk-install.md)。

## 如果已安装但未生效

仪表盘会说明原因：

- **需要 SuperKey**——APatch/FolkPatch 无法用受信任的 `su` 令牌加载它。把
  SuperKey 保存到 **Settings → Security** 并重启（或点 Refresh）。重装 ZIP 无济
  于事。
- **需要运行时**——内核里没有 KernelPatch 运行时；安装一个（见上）。
- **检测到独立安装**——加载了裸的 `vpnhide.kpm`，而模块 ZIP 未安装。移除那个独立
  条目，从 Modules 界面安装完整的 `vpnhide-kpm.zip` 并重启。
- **与内核模块冲突**——`.ko` 和 KPM 都已安装；它们挂钩相同函数、可能使设备死机，
  因此 KPM 主动退让。卸载其中之一。**切勿同时保留两者生效。**

若加载失败，用 **Settings → Debugging → Collect debug log** 把开机时的输出附到
报告中——见[收集调试报告](collect-report.md)。
