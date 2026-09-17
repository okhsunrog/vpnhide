# 首次安装

VPN Hide 由几部分协同工作：这个应用（选择器）、一个**原生**后端、通过 LSPosed 的
**Java** 层，以及可选的 **Ports** 模块。请先查看[要求与兼容性](requirements.md)。

以下步骤描述推荐的 Java + Native 配置。可以省略某层，但其检测路径将不受覆盖；
Ports 为可选。见[要求](requirements.md)。

## 步骤

1. **安装应用**——从项目发布页下载选择器 APK。你正在其中阅读这份帮助的仪表盘就是它。
2. **授予 root 权限。**VPN Hide 需要 root 才能管理隐藏。打开应用并在 root 管理器
   （Magisk / KernelSU / APatch）的弹窗中授权。若错过弹窗，应用会显示 **Root
   access required** 界面——授予后点 **Check again**。
3. **安装一个原生后端。** 仪表盘会针对你的设备推荐最合适的一个。通过 root 管理器的
   **模块**页面安装它的模块（ZIP）——Magisk、KernelSU、APatch 或 FolkPatch。只装一个
   原生后端；两个同时生效可能冲突。
4. **启用 Java 层。** 在 LSPosed / LSPosed-Next / Vector 中启用 **VPN Hide**，并把
   **系统框架（System Framework）**加入它的作用域。没有这个作用域，Java 钩子不会挂载。
5. **（可选）安装 Ports**，如果你需要隐藏本地回环端口。
6. **重启**，让原生和 Java 模块加载。

## 确认是否生效

打开**仪表盘**。每个已安装的模块都应显示 **Active（生效）**；当 VPN 开启且其检查通过
时，顶部会显示 **VPN 已隐藏**。如果某模块提示「重启以激活」或「未生效」，按屏幕提示操作。
常见原因：没加作用域、模块未重启，或同时装了两个原生后端。

然后前往[设置隐藏](configure-hiding.md)。

## 官方下载

从[项目发布页](https://github.com/okhsunrog/vpnhide/releases)获取 APK 和模块 ZIP。按设备推荐选择原生 ZIP；内核 ZIP 并非所有手机通用。
