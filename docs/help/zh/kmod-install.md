# 安装内核模块

内核模块（即 `.ko`，仪表盘上显示为 **Kmod**）是 GKI 内核上的默认 Native 后端。
它挂钩在内核空间，因此从不触碰目标应用的进程内存，反篡改检测也看不到它。请先确认
它适合你的设备——见[该选哪个 Native 后端](choosing-native.md)。

## 匹配 GKI 版本

`.ko` 按 **GKI 代次**分别构建。你要安装与内核 KMI（Kernel Module Interface）标签
匹配的 ZIP——例如 `android14-6.1`。该标签是内核版本字符串的一部分，标识的是内核
对模块的二进制接口，**不是**你的 Android 系统版本，所以标签里的“android14”并不
表示你在用 Android 14。

**仪表盘**会读取你的内核并指出要下载的确切文件（例如
`vpnhide-kmod-android14-6.1.zip`）。若无法从版本字符串区分你的 GKI 分支，它会先
给出一个“先试这个”，再给一个备选——万一第一个加载不了就用备选。

## 步骤

1. 下载仪表盘指定的 ZIP。
2. 在你的 root 管理器（Magisk / KernelSU / APatch）中打开 **Modules → Install
   from storage**，选择该 ZIP。
3. **重启**——模块在开机时加载。
4. 打开**仪表盘**；Native 后端应显示 **Active**。

目标应用在 VPN Hide 应用里选择，无需手动改任何文件。

## 更新或移除

已加载的 `.ko` 会驻留到重启为止。要更新或移除它，请在 root 管理器的 **Modules**
界面操作（更新 ZIP 或停用模块）并**重启**以生效。不支持普通的 `rmmod`。

## 如果加载不了

仪表盘会告诉你具体属于哪种情况：

- **Installed, wrong GKI variant**——你刷了另一代次的 ZIP。安装仪表盘指定的
  那个。
- **Installed, did not load — try the other GKI variant**——GKI 分支不明确；安装
  仪表盘建议的备选版本。
- **Installed, kernel not supported** / **kernel has no kprobes**——此内核无法
  加载 `.ko`。若你的内核家族受支持，改用 [KPM 后端](kpm-install.md)，否则用
  [Zygisk 模块](zygisk-install.md)。
- **Installed, kernel rejects unsigned modules**——启用了模块签名校验，`.ko`
  无法插入。使用 KPM（通过 KernelPatch 加载，而非模块加载器）或 Zygisk。

若模块已加载但仪表盘显示只装上了部分挂钩，说明在这台设备的内核上，此后端有几个
检测面没有覆盖；重装也不会改变。见[模块已安装但未生效](module-not-active.md)。
