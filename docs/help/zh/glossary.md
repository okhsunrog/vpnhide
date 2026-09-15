# 术语表

本指南中所用术语的简短定义。

- **后端（Backend）**——执行隐藏的某个组件。VPN Hide 有 **Java** 后端（LSPosed）
  和 **native** 后端（kmod、KPM、Zygisk 或内置）。见
  [该选哪个 Native 后端](choosing-native.md)。
- **Native**——在 Java 框架以下进行隐藏：内核或 libc 级别，覆盖直接系统调用、
  接口列表、netlink 和 `/proc/net`。
- **Java 层 / LSPosed**——用于挂钩 `system_server` 的框架，好在应用看到之前，就把
  VPN 从 Android 的 Java 网络 API 中剥除。
- **挂钩（Hook）**——单个拦截点。一个后端会安装多个挂钩，每个覆盖一条检测路径。
- **目标（Target）**——你选中用来*对其*隐藏 VPN 的应用（银行、商店），与之相对
  的是你的 VPN 客户端，即被隐藏的对象。
- **角色（J / N / A / P）**——Java / Native / Apps / Ports，可为每个目标开启的
  四项。见[设置隐藏](configure-hiding.md)。
- **UID**——Android 分配给每个应用的 Linux 用户 ID。隐藏按 UID 应用——正因如此，
  一个应用被过滤，而系统和你的 VPN 客户端不会。
- **包（Package）**——应用的标识符（如 `com.example.bank`）。角色按包名保存。
- **资料（Profile）**——设备上独立的用户空间：主资料、**工作资料**、**克隆应用**
  或第二空间。只在主资料中安装 VPN Hide——见[工作资料](work-profiles.md)。
- **GKI**——Google 的通用内核映像。内核模块按 GKI 代次分别构建。
- **KMI**——Kernel Module Interface（内核模块接口），标识内核对模块二进制接口的
  标签（如 `android14-6.1`）。你据它匹配 `.ko`——见[安装内核模块](kmod-install.md)。
- **KernelPatch / SuperKey**——KPM 所需的运行时，以及 APatch/FolkPatch 用来授权
  它的密钥——见[安装 KPM 后端](kpm-install.md)。
- **Localhost / loopback**——设备自身的 `127.0.0.1` / `::1` 地址，本地代理或 VPN
  守护进程可能在此监听。**Ports** 角色按应用阻断它。
- **路由表 / 路由**——内核关于流量去向的列表；VPN 会添加一条经其隧道的路由，这
  可能暴露它。
- **分流（Split tunneling）**——在你的 VPN 客户端里选择哪些应用走隧道、哪些直连
  ——见[直连还是走隧道](split-tunneling.md)。
- **SELinux**——Android 的强制访问控制。若被设为 Permissive，几条本由内核封堵的
  检测路径会重新开放——见[每个结果是什么意思](check-result-meanings.md)。
