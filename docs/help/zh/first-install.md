# 首次安装

安装**同一发布/构建**的 VPN Hide APK 和**一个完整 Core ZIP**。Core 包含原生候选
实现和 Ports。Java/Apps 覆盖还需要 LSPosed 或兼容实现。先看
[要求与兼容性](requirements.md)。

## 步骤

1. **安装应用**，使用配套发布中的 APK。
2. 在 root 管理器的模块页面**安装 `vpnhide-core.zip`**，保持 Core 启用。APK 与 Core 通信，
   自身不请求 root。
3. **由 Core 自动选择 Native。**通用 ZIP 已包含 payload。见[后端选择](choosing-native.md)
   和[KPM 要求](kpm-install.md)。
4. 在 LSPosed / Vector 中**启用 Java 层**，将 **System Framework** 加入 VPN Hide 作用域。
5. **Ports 可选：**之后给应用分配该角色；它已包含在 Core 中。
6. **重启并解锁**设备。

升级时，在替代安装准备并验证完成前保留配置和可用安装。旧的独立模块按 Core 的迁移
提示处理。

## 确认是否生效

打开仪表盘，查看 Core 就绪状态和保留的诊断结果。它们不依赖外部 VPN 是否开启，
也不依赖此 APK 是否包含在 VPN 的分流列表中。等待、失败或不完整的证据都不是保护
成功的结果。之后按[第一个应用](first-setup.md)选择角色并点击**保存**。

## 官方下载

从[项目发布页](https://github.com/okhsunrog/vpnhide/releases)或提供给你的同一构建中获取
配套 APK 和完整 `vpnhide-core.zip`。仅含 daemon 的 base 归档是内部构建输入，不是完整
安装包。
