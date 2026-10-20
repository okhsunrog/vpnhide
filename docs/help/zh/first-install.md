# 首次安装

VPN Hide 由几部分协同工作：这个应用（选择器）、负责所有 root 工作的 **VPN Hide Core**
模块、一个**原生**后端，以及通过 LSPosed 的 **Java** 层。本地回环端口隐藏（**Ports**）
属于核心模块。请先查看[要求与兼容性](requirements.md)。

以下步骤描述推荐的 Java + Native 配置。可以省略某层，但其检测路径将不受覆盖；
Ports 为可选。见[要求](requirements.md)。

## 步骤

1. **安装应用**——从项目发布页下载选择器 APK。你正在其中阅读这份帮助的仪表盘就是它。
2. **安装核心模块。**通过 root 管理器（Magisk / KernelSU / APatch）的**模块**页面安装与应用
   **同一版本**的 `vpnhide-core.zip`，并保持启用。应用本身从不请求 root，而是与核心模块通信。
   在核心运行之前（第 6 步重启之前），应用会显示 **“VPN Hide Core 未运行”**；来自其他版本的
   核心会显示 **“VPN Hide Core 版本不匹配”**。从 1.x 升级？如有旧的 **VPN Hide (Ports)** 和
   **VPN Hide (Built-in kernel)** 模块，请卸载。
3. **安装一个原生后端。** 仪表盘会针对你的设备推荐最合适的一个。通过 root 管理器的
   **模块**页面安装它的模块（ZIP）——Magisk、KernelSU、APatch 或 FolkPatch。只装一个
   原生后端：多于一个时，核心一个也不加载。
   安装说明：[kmod](kmod-install.md) · [KPM 与 KernelPatch 配置](kpm-install.md) ·
   [Zygisk](zygisk-install.md)。不确定时，请查看[选择后端](choosing-native.md)，
   其中也介绍了内置 VPN Hide 的内核。
4. **启用 Java 层。** 在 LSPosed / LSPosed-Next / Vector 中启用 **VPN Hide**，并把
   **系统框架（System Framework）**加入它的作用域。没有这个作用域，Java 钩子不会挂载。
5. **（可选）[Ports](ports.md)**——无需安装，之后给应用分配 Ports 角色即可。
6. **重启**，让核心启动并加载原生后端，Java 层随之加载。

## 确认是否生效

打开**仪表盘**。每个已安装的模块都应显示 **Active（生效）**；当 VPN 开启且其检查通过
时，顶部会显示 **VPN 已隐藏**。如果某模块提示「重启以激活」或「未生效」，按屏幕提示操作。
常见原因：没加作用域、模块未重启、未安装核心模块，或同时装了两个原生后端。

然后前往[设置隐藏](configure-hiding.md)。

## 官方下载

从[项目发布页](https://github.com/okhsunrog/vpnhide/releases)获取 APK、`vpnhide-core.zip` 和后端 ZIP。按设备推荐选择原生 ZIP；内核 ZIP 并非所有手机通用。
