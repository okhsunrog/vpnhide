<p align="center">
  <img src="assets/logo.png" width="200" alt="VPN Hide" />
</p>

<h1 align="center">VPN Hide</h1>

<p align="center">对选定应用隐藏活动的 Android VPN 连接。</p>

<p align="center">
  <a href="https://github.com/okhsunrog/vpnhide/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/okhsunrog/vpnhide/ci.yml?label=CI" alt="CI"></a>
  <a href="https://github.com/okhsunrog/vpnhide/releases/latest"><img src="https://img.shields.io/github/v/release/okhsunrog/vpnhide" alt="Release"></a>
  <a href="https://github.com/okhsunrog/vpnhide/releases"><img src="https://img.shields.io/github/downloads/okhsunrog/vpnhide/total" alt="Downloads"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="License"></a>
</p>

<p align="center"><strong><a href="README.md">Русская версия</a> · <a href="README.en.md">English version</a></strong></p>

> 中文翻译由社区提供，欢迎母语者审阅与纠错。如发现措辞问题，欢迎在 [GitHub Issues](https://github.com/okhsunrog/vpnhide/issues) 反馈。

## 相比同类方案，vpnhide 好在哪？

现有模块，如 [NoVPNDetect](https://bitbucket.org/yuri-project/novpndetect) 和 [NoVPNDetect Enhanced](https://github.com/BlueCat300/NoVPNDetectEnhanced)，只覆盖 **Java API** 检测，并通过 Xposed 在**目标应用进程内部**挂钩。这种做法有两个致命问题：

1. **对防篡改不隐蔽** —— 任何带内存注入检测的应用都会发现 Xposed 钩子并拒绝工作。NoVPNDetect Enhanced 的作者明确表示：*“如果目标应用有 LSPosed 防护或内存注入检测，模块将无法工作。例如 MirPay、T-Bank。”*
2. **没有原生层覆盖** —— 使用 C/C++ 代码、跨平台框架（Flutter、React Native）或直接系统调用的应用，可以通过 `ioctl`、`getifaddrs`、netlink 套接字和 `/proc/net/*` 检测 VPN。这些向量被仅 Java 的钩子完全遗漏。

vpnhide 用分层架构同时解决这两个问题：

**第 1 层 —— Java API（lsposed 模块）：** 挂钩的是 `system_server`，而非目标应用。`NetworkCapabilities`、`NetworkInfo` 和 `LinkProperties` 在 Binder 层、在数据到达应用进程*之前*就被过滤。应用通过 IPC 收到干净的数据 —— 没有注入其进程，防篡改无从检测。同一模块还在 `PackageManager` 层对观察者应用隐藏选定应用（**Apps** 角色）。

**第 2 层 —— 原生（kmod、KPM 或 Zygisk）：** 覆盖原生检测路径。同一时间应恰好有一个原生后端处于活动状态：
- **kmod**（推荐用于受支持的 GKI 内核）—— 内核级 `kprobe`/`kretprobe` 钩子。过滤 `ioctl`、`getifaddrs`/netlink 的接口、地址、路由和策略规则转储，并在套接字状态改变前对隐藏接口拒绝 `SO_BINDTODEVICE` / `SO_BINDTOIFINDEX`。在目标进程中零足迹：无库注入，无可检测之物。
- **KPM** —— 一个 KernelPatch 模块，实现与之相同的 11 个逻辑内核钩子，无需针对特定 GKI 变体的 `.ko`。适用于旧的/非 GKI 的 4.14 / 4.19 / 5.4 内核，以及 `.ko` 无法加载的情况。需要 KernelPatch 运行时：APatch 或 KPatch-Next-Module。
- **Zygisk** —— 当无法使用内核级后端时的回退方案。它的 `libc.so` 内联钩子包含尽力而为的 `setsockopt` 过滤，运行在目标进程内部，可被直接系统调用绕过，因此银行和反欺诈应用可能会检测到它。对于这类应用，请关闭原生并依赖 Java 层。

**第 3 层 —— 端口模块（portshide）：** 一个独立的 Magisk 模块。它阻止选定应用访问 `127.0.0.1` / `::1`（通过 iptables），使其无法通过本地绑定的 VPN / 代理守护进程的开放端口来检测它们（**Ports** 角色）。

使用 LSPosed + 内核级原生后端（kmod 或 KPM）时，目标应用的进程完全不受触碰 —— 没有 Xposed，没有内联钩子，没有被修改的内存区域。正因如此，vpnhide 能与主动检测并封锁基于 Xposed 模块的银行和政务类应用协同工作。

## vpnhide 隐藏什么

vpnhide 对选定应用隐藏三样东西，全部通过四个 **J / N / A / P** 角色（Java、Native、Apps、Ports）按应用配置：

1. **接口隐藏** —— 主要目标。它从原生 API（`ioctl`、`getifaddrs`、netlink、`/proc/net/*`、`NetworkInterface`）中移除 VPN 接口和路由，阻止将套接字绑定到隐藏接口，并净化 Java API（`NetworkCapabilities`、`NetworkInfo`、`LinkProperties`）。它由两个角色共同提供 —— **Java (J)** 与 **Native (N)** —— 可独立开关。
2. **端口隐藏** —— 为选定应用阻断本地回环访问，使其无法通过探测本地端口来检测 Clash、sing-box、V2Ray、Happ 等工具（**Ports (P)** 角色）。
3. **应用隐藏** —— 对选定的观察者应用隐藏选定的已安装应用。可用于对抗软件包可见性检查，例如某应用试图判断是否安装了 VPN 或代理客户端时（**Apps (A)** 角色）。

## 我需要哪些模块？

你始终需要 **VPN Hide 应用**（`vpnhide.apk`）+ 用于 Java 层的 LSPosed/Vector + 恰好一个用于原生隐藏的原生后端。若你想要本地回环端口拦截，应用还可选用可选的端口模块：

- **`kmod`**（稳定默认）—— 完全在进程外，对防篡改不可见。需要受支持的 GKI 内核：5.10、5.15、6.1、6.6 或 6.12。
- **`KPM`** —— 用于 4.14 / 4.19 / 5.4 以及 `.ko` 不适配的其他情况的内核级后端。需要 APatch 或 KPatch-Next-Module。
- **`Zygisk`** —— 当 kmod/KPM 不可用，或你不想安装 KernelPatch 运行时时的回退方案。
- **`portshide`**（可选）—— 若你想阻止选定应用探测本地回环端口，请安装它。

不要同时安装多个原生后端。若安装了多个，应用会按优先级选择活动的那个：kmod，其次 KPM，再次 Zygisk；请卸载不使用的模块。

分步说明见[安装](#安装)。

## 安装

从 [Releases](https://github.com/okhsunrog/vpnhide/releases) 下载最新发行版。

### 第 1 步 —— VPN Hide 应用 + LSPosed

1. 将 `vpnhide.apk` 作为普通应用安装
2. 在 LSPosed 管理器中，启用 VPN Hide 模块并将 **“系统框架”** 添加到其作用域
3. 重启设备（必需 —— LSPosed 钩子在开机时注入 `system_server`，因此模块必须在 `system_server` 启动前处于活动状态）
4. 打开 VPN Hide 应用并授予 Root 权限（Magisk 通常会自动提示；在 KernelSU/KernelSU-Next/APatch 上，请在管理器中授权）

### 第 2 步 —— 用于接口隐藏的原生模块

打开 VPN Hide 应用。**概览**标签页会检测你的设备和内核，并告诉你应安装哪个原生后端：

- 对于受支持的 GKI 内核，它会推荐特定的 kmod 文件，例如 `vpnhide-kmod-android14-6.1.zip`。
- 对于旧的/非 GKI 的 4.14 / 4.19 / 5.4 内核，它会推荐 `vpnhide-kpm.zip`。若尚未检测到 KernelPatch 运行时，应用会要求你先安装 KPatch-Next-Module，或使用 Zygisk 作为回退。
- 对于其他内核，它会推荐 `vpnhide-zygisk.zip`。

安装推荐的模块：
- **kmod：** 通过 KernelSU-Next / KernelSU / Magisk 管理器 → 模块 → 从本地安装。
- **KPM：** 安装 `vpnhide-kpm.zip`；在 APatch/FolkPatch 下，若运行时不提供受信任的 KernelPatch `su` 令牌，应用可能会要求你在 **设置 → 安全** 中保存 SuperKey 以便开机时激活。在 Magisk、KernelSU 和 KernelSU-Next 下，若尚未安装 KPatch-Next-Module，请先安装它。
- **Zygisk：** 通过 KernelSU-Next、KernelSU 或 Magisk 管理器 → 模块。

安装原生模块后请重启设备。

### 第 3 步 —— 可选：安装端口模块

若你想要本地回环端口拦截，请通过 KernelSU-Next 或 Magisk 管理器安装 `vpnhide-ports.zip`。

此模块独立于原生后端，仅在应用中使用 **Ports** 角色时才需要。

### 第 4 步 —— 配置隐藏

打开 VPN Hide 应用 → **隐藏** 标签页。

每个应用行都有角色：

- **Java** —— 在 LSPosed/system_server 层通过 Android Java API 隐藏 VPN。
- **Native** —— 活动的原生后端：kmod、KPM 或 Zygisk。VPN Hide 只保存一份原生选择；只有活动后端会生效。
- **Apps** —— 该应用成为观察者，应收到经过净化的 PackageManager 视图，其中选定的 VPN/代理应用被隐藏。
- **Ports** —— 阻止该应用访问本地回环端口。

在设置中可将简短的 **J / N / A / P** 标记切换为完整角色标签。对于 Java、Native 和 Ports，标签旁的设置图标可打开单项钩子或端口范围设置。

更改后请点按保存。

#### 该配置哪个应用

角色配置在**探测方应用**上——也就是你要对其隐藏 VPN 的那个（银行、政务、电商）。VPN 应用本身在这里不需要配置：它是被隐藏的一方，那份名单在“设置 → 隐藏 VPN 应用”里。

常见场景：

| 情况 | 需要开启 |
|---|---|
| 银行应用不该看到 VPN 已连接 | 在**银行应用**上：Java + Native |
| 银行应用还会扫描已安装应用列表 | 再加上银行应用的 **Apps**。然后在“设置 → 隐藏 VPN 应用”中确认你的 VPN 在列表里：声明了 VpnService 的应用会被自动识别，其余需要手动添加 |
| 银行应用专门针对 Zygisk 报警 | 关闭 Native，只留 Java。GKI 设备上更建议换成 kmod 或 KPM——它们在进程内部不可见 |
| 应用探测本地回环代理端口（127.0.0.1:1080 之类） | 在该应用上再加 **Ports**（需要已安装 ports 模块） |

举例：要让某个银行应用既看不到 VPN，也看不到已安装的 WireGuard，给银行应用开 **J + N + A**，并确认 WireGuard 在隐藏名单中。WireGuard 自身不需要任何角色。


Java 和内核级原生后端（kmod/KPM）会立即生效。Zygisk 钩子和端口规则需在选定应用被强制停止并重新打开后才会被读取。

> **注意：** 某些应用在为其启用原生时会检测到 Zygisk 钩子。请对这类应用关闭原生并依赖 Java 层，或改用 kmod/KPM。

<details>
<summary><b>Shell 配置（进阶）</b></summary>

用户管理的配置位于 `/data/system/vpnhide_config.json`。编辑该 JSON，然后为已安装的模块运行激活器：

```sh
su -c /data/adb/modules/vpnhide_kmod/activator
su -c /data/adb/modules/vpnhide_kpm/activator
su -c /data/adb/modules/vpnhide_zygisk/activator
su -c /data/adb/modules/vpnhide_ports/activator
```

只运行实际已安装模块的激活器。LSPosed 直接从 `system_server` 读取该 JSON；它不需要激活器。旧的 `/data/adb/vpnhide_*` `targets.txt` 文件是 1.0 之前版本的配置。它们不再是用户配置：应用会将其一次性导入该 JSON 并删除。

</details>

<details>
<summary><b>手动查询 GKI（若你想自行挑选 kmod 文件）</b></summary>

1. 在手机上进入 **设置 → 关于手机**，找到 **内核版本** 一行。它形如 `6.1.75-android14-11-g...`
2. 你需要从该字符串中取两部分：内核版本（`6.1`）和 android 代号（`android14`）。二者合起来构成你的 GKI 代号：`android14-6.1`
3. 从发行版下载匹配的文件：`vpnhide-kmod-android14-6.1.zip`

或者运行 `adb shell uname -r` 查看内核版本字符串。

> **重要：** 内核字符串中的 `android14` 并不是你的 Android 版本，而是内核代号。例如，从 Pixel 6 到 9a 全都使用 `android14-6.1` 内核，无论它们运行 Android 14 还是 15。

</details>

## 截图

| 概览 —— VPN 已隐藏 | 隐藏 —— 单一应用列表 | 工作原理 |
|:-:|:-:|:-:|
| <img src="assets/screenshots/dashboard-hidden.png" width="250"> | <img src="assets/screenshots/hiding-list.png" width="250"> | <img src="assets/screenshots/hiding-help.png" width="250"> |

| 统计 | 按钩子细分 | 按应用选择钩子 |
|:-:|:-:|:-:|
| <img src="assets/screenshots/statistics.png" width="250"> | <img src="assets/screenshots/statistics-breakdown.png" width="250"> | <img src="assets/screenshots/hook-picker.png" width="250"> |

| 诊断 | 设置 | 社区 |
|:-:|:-:|:-:|
| <img src="assets/screenshots/diagnostics-native.png" width="250"> | <img src="assets/screenshots/settings.png" width="250"> | <img src="assets/screenshots/community.png" width="250"> |

## 验证

应用内置诊断系统，可自动捕捉大多数配置问题。

**概览**（每次启动应用时运行）：
- 模块和后端状态（已安装、活动、版本、目标数量）
- LSPosed 配置校验 —— 读取 LSPosed 数据库，验证 VPN Hide 已启用、系统框架在作用域内，且没有多余应用被纳入作用域（一个常见的错误配置）
- 版本不匹配检测 —— 将已安装模块版本与运行中的应用版本对比，并明确告诉你需要更新什么
- 原生后端推荐 —— 检测你的内核并映射到正确的 kmod、KPM 或 Zygisk 产物
- 实时隐藏检查（当 VPN 活动时）—— 运行 13 项原生检查和 12 项 Java API 检查，验证 VPN 是否确实被隐藏

发现的任何问题都会以可操作的卡片形式显示，附带具体说明。

**统计** 标签页 —— 按应用细分哪些应用探测 VPN 以及如何探测，显示每个应用运行了哪些检查（由活动后端上报的计数）。

**设置 → 诊断**（详细诊断）—— 对全部 25 项检查逐项细分，给出各自的 PASS/FAIL 结果。当概览显示隐藏不完整时，可用于排查问题。

## 组成部分

| 目录 | 是什么 | 如何工作 |
|---|---|---|
| **[kmod/](kmod/)** | `.ko` 内核模块 + KPM 后端（C） | 两个内核级原生后端：使用 `kretprobe` 的稳定 GKI `.ko`，以及使用 KernelPatch 内联钩子的 KPM。两者在目标应用进程中均零足迹；只应有一个处于活动状态。（[详情](kmod/README.md)，[KPM](kmod/kpm/README.md)） |
| **[lsposed/](lsposed/)** | LSPosed 模块 + 应用（Kotlin + Rust） | 在 `system_server` 中挂钩 `writeToParcel`，实现按 UID 的 Binder 过滤。APK 提供概览（模块状态、版本检查、LSPosed 配置校验、安装建议）、用于 Java / Native / Apps / Ports 角色的隐藏标签页，以及诊断。（[详情](lsposed/README.md)） |
| **[portshide/](portshide/)** | 端口模块（Shell + iptables） | 阻止选定应用访问 `127.0.0.1` / `::1`，使本地绑定的 VPN / 代理守护进程免于本地回环端口探测。（[详情](portshide/README.md)） |
| **[zygisk/](zygisk/)** | Zygisk 模块（Rust） | 在目标应用进程中内联挂钩 `libc.so`。当内核级后端不可用时的回退方案。（[详情](zygisk/README.md)） |

## 检测覆盖

| # | 检测向量 | SELinux | kmod | KPM | Zygisk | LSPosed |
|---|---|---|---|---|---|---|
| 1 | tun0 上的 `ioctl(SIOCGIFFLAGS)` | | x | x | x | |
| 2 | `ioctl(SIOCGIFNAME)` 由索引解析名称 | | x | x | x | |
| 3 | `ioctl(SIOCGIFMTU)` MTU 指纹 | | x | x | x | |
| 4 | `ioctl(SIOCGIFCONF)` 接口枚举 | | x | x | x | |
| 5 | 其他所有 `SIOCGIF*`（INDEX、HWADDR、ADDR 等） | | x | x | x | |
| 6 | `getifaddrs()`（内部使用 netlink） | | x | x | x | |
| 7 | netlink `RTM_GETLINK` 转储 | | x | x | x | |
| 8 | netlink `RTM_GETADDR` 转储（IPv4 + IPv6） | | x | x | x | |
| 9 | netlink `RTM_GETROUTE` 转储 | | x | x | x | |
| 10 | netlink `RTM_GETRULE` 策略规则 | | x | x | | |
| 11 | 指向 VPN 服务器的公网 `/32` 或 `/128` 主机路由 | | x | x | | |
| 12 | `SO_BINDTODEVICE` / `SO_BINDTOIFINDEX` | | x | x | libc | |
| 13 | `/proc/net/route` | 已阻断 | x | x | x | |
| 14 | `/proc/net/ipv6_route` | 已阻断 | x | x | x | |
| 15 | `/proc/net/if_inet6` | 已阻断 | | | x | |
| 16 | `/proc/net/tcp`、`tcp6` | 已阻断 | | | x | |
| 17 | `/proc/net/udp`、`udp6` | 已阻断 | | | | |
| 18 | `/proc/net/dev` | 已阻断 | | | | |
| 19 | `/proc/net/fib_trie` | 已阻断 | | | | |
| 20 | `/sys/class/net/tun0/` 及 `/proc/sys/net/*/{conf,neigh}/tun0` | 已阻断 | 可选启用 | | | |
| 21 | `NetworkCapabilities`（hasTransport、NOT_VPN、transportInfo） | | | | | x |
| 22 | `NetworkInfo`（getType、getTypeName） | | | | | x |
| 23 | `ConnectivityManager.getActiveNetwork()` | | | | | x |
| 24 | `ConnectivityManager.getAllNetworks()` + VPN 扫描 | | | | | x |
| 25 | `LinkProperties`（interfaceName） | | | | | x |
| 26 | `LinkProperties`（经 VPN 接口的路由） | | | | | x |
| 27 | `NetworkInterface.getNetworkInterfaces()` | | x | x | x | |
| 28 | 经 Java `FileInputStream` 读取 `/proc/net/route` | 已阻断 | x | x | x | |

**已阻断** = 在原厂 enforcing 构建（Android 10+）上，SELinux 通常会拒绝不受信任的应用访问该 `/proc/net/*` / `/sys` 文件。但**不同设备和 ROM 的 SELinux 策略配置各不相同**（OEM 和第三方 ROM、`permissive` 构建），因此只有表中明确标注的覆盖才应视为 vpnhide 的保护。

**libc** = Zygisk 的尽力而为覆盖：直接系统调用会绕过内联钩子。

**可选启用** = `.ko` 的文件系统隐藏功能默认关闭。请在设置中启用并重启；关闭时其全局 VFS 探测不会被安装。

重要：`ioctl` 和 netlink 转储对普通应用无需 SELinux 帮助即可使用；在 Linux 5.7+ 上，首次套接字接口绑定也是如此。这正是 RKNHardering 等检测器通过 netlink 绕过 `/proc/net/route` 拒绝的方式（见 [issue #86](https://github.com/okhsunrog/vpnhide/issues/86)）。内核级后端（kmod/KPM）覆盖上表标注的原生路径，且在目标进程中无足迹。Zygisk 仅覆盖经 libc 路由的调用；直接的原始系统调用会绕过其钩子。在较旧的内核上，内核本身会拒绝非特权的接口绑定。其余的要么在原厂上常被 SELinux 阻断（视设备而定），要么经由 Java API 并由 LSPosed 覆盖。

KPM 实现与 `.ko` 相同的 11 个逻辑内核钩子，而完整的向量图记录了在较旧内核上剩余的 ABI 与行为差异。

完整的向量图 —— 按层细分、SELinux 注意事项和已知缺口 —— 位于 [docs/detection-vectors.md](docs/detection-vectors.md)。

## 从源码构建

- **kmod**：`./kmod/build.py --kmi android14-6.1`（或 `--all`）—— 通过 podman/docker 自动拉起 DDK 容器。完整指南：[kmod/BUILDING.md](kmod/BUILDING.md)。
- **KPM**：`python3 kmod/kpm/build.py` —— 通过 KernelPatch 子模块构建通用的 `vpnhide-kpm.zip`。详情：[kmod/kpm/README.md](kmod/kpm/README.md)。
- **zygisk**：`cd zygisk && ./build.py`（Rust + NDK + cargo-ndk）
- **lsposed**：`cd lsposed && ./gradlew assembleDebug`（JDK 17 + Rust + NDK + cargo-ndk）

### 给受困于 Windows 的贡献者的说明

如果你在 Windows 上，构建某些子项目会有一些不便。

**lsposed**：在 Android Studio 中可正常构建。

**portshide**：`cd .\portshide\; python .\build-zip.py` 可正常运行。

对于 kmod 和 zygisk，你（很遗憾地）需要安装 [Docker for Windows](https://docs.docker.com/desktop/setup/install/windows-install/)。

**kmod**：`python .\kmod\build.py --kmi android14-6.1` —— 脚本会自动识别 Docker 并拉取与 CI 相同的 `ddk-min` 镜像。

**KPM**：请在 Linux 或 WSL 下构建。脚本需要 POSIX 工具、`make`/`clang`、KernelPatch 子模块和 Android NDK；未记录原生 Windows 构建路径。

**zygisk**：
```powershell
docker run --rm -it -v "${PWD}:/workspace" -v "vpnhide_cargo_cache:/usr/local/cargo/registry" -w /workspace ghcr.io/okhsunrog/vpnhide/ci:latest bash -c 'cd zygisk && python3 ./build.py'
```
`zygisk` 无法在 Windows 上直接构建的原因是：其 `zygisk-api` 依赖包含一个名为 `aux.rs` 的文件。Cargo 使用 `libgit2` 进行 git 操作，而 `libgit2` 拒绝创建名称*包含* Windows 保留设备名（`AUX`、`CON`、`NUL`……）的文件。你会得到：`cannot checkout to invalid path 'src/aux.rs'; class=Checkout (20)`。[有人报告](https://superuser.com/a/1929659)称，某次 Windows 更新使得可以创建**带扩展名**的、包含保留字的文件，但 `libgit2` 尚未更新以放宽该限制。

## 已验证对抗

- [RKNHardering](https://github.com/xtclovver/RKNHardering/) —— 所有检测向量均干净
- [YourVPNDead](https://github.com/loop-uh/yourvpndead) —— 所有检测向量均干净

两者均实现了俄罗斯数字发展部官方的 VPN/代理检测方法论（[来源](https://t.me/ruitunion/893)）。

## 分应用代理（Split tunneling）

可与分应用代理的 VPN 配置正确协同工作。仅目标列表中的应用会受影响。

强烈建议将分应用代理与 VPN Hide 一起使用。

将设备上报的公网 IP 与外部检测器对比的检测类应用应留在隧道之外 —— 它们的流量应走运营商网络，而非 VPN。

## 威胁模型

vpnhide 对特定应用隐藏活动的 VPN。它并非为以下用途设计：
- 隐藏 Root 或第三方 ROM 的存在
- 绕过 Play Integrity
- 欺骗服务端检测（DNS 泄漏、IP 黑名单、延迟/TLS 指纹）

## 已知限制

- `kmod` 需要带 `CONFIG_KPROBES=y` 的受支持 GKI 内核（Android 12+ 设备上为标准配置）
- KPM 需要 KernelPatch 运行时（APatch 或 KPatch-Next-Module）；不要将 KPM 与 `.ko` 一起安装
- `lsposed` 需要 LSPosed、LSPosed-Next 或 Vector
- `zygisk` 仅支持 arm64
- 直接的 `svc #0` 系统调用会绕过 Zygisk 的 libc 钩子 —— 为此请使用内核级后端（kmod 或 KPM）
- 服务端检测在客户端无法解决 —— 请使用分应用代理

## 支持项目

vpnhide 免费，无广告、无遥测。不会有付费功能，捐赠不会解锁任何内容。

应用内的 **设置 → 支持项目** 中也有同样的列表，点按地址即可复制。

| 币种 | 网络 | 地址 |
| --- | --- | --- |
| USDT | Tron (TRC20) | `TMskx2wKmPg11VYvHoS93vUQGm7yhcetUU` |
| BTC | Bitcoin | `bc1pmt9u6nux4x7n86zknwdgt9v02lah2tu6d983ak2prc5cwt8hsetq82ganh` |
| GRAM | The Open Network | `UQADYTtMBQdZvmNNEX02R9sACpdnXKlPV8RbuFrxo7JFBRGS` |
| LTC | Litecoin | `MBLKJfPNANH3U41UPJFtha7EPJGdbiW5dZ` |

## 许可证

MIT。见 [LICENSE](LICENSE)。

内核模块声明了 `MODULE_LICENSE("GPL")`，这是 Linux 内核在运行时解析 `EXPORT_SYMBOL_GPL` 符号所要求的。

## Star 历史

<a href="https://www.star-history.com/?type=date&repos=okhsunrog%2Fvpnhide">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=okhsunrog/vpnhide&type=date&theme=dark&legend=top-left&sealed_token=L6VLoQFGmusCfaI01irFbE2MJWoOX9V4Z66YMzG6z0vD-xjku8IZX4jnDHFYeAjjEne48AgxfoSExLa90tYlqeYq7E32T0DGbdKrR8UTyp0zVCfDeKGdCIku_20sKVi9WBuO4aqa3nBKDnKepezie3AC67kmr-2mazo76SIUyXWpRp8Lb038KZtafha8" />
    <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=okhsunrog/vpnhide&type=date&legend=top-left&sealed_token=L6VLoQFGmusCfaI01irFbE2MJWoOX9V4Z66YMzG6z0vD-xjku8IZX4jnDHFYeAjjEne48AgxfoSExLa90tYlqeYq7E32T0DGbdKrR8UTyp0zVCfDeKGdCIku_20sKVi9WBuO4aqa3nBKDnKepezie3AC67kmr-2mazo76SIUyXWpRp8Lb038KZtafha8" />
    <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=okhsunrog/vpnhide&type=date&legend=top-left&sealed_token=L6VLoQFGmusCfaI01irFbE2MJWoOX9V4Z66YMzG6z0vD-xjku8IZX4jnDHFYeAjjEne48AgxfoSExLa90tYlqeYq7E32T0DGbdKrR8UTyp0zVCfDeKGdCIku_20sKVi9WBuO4aqa3nBKDnKepezie3AC67kmr-2mazo76SIUyXWpRp8Lb038KZtafha8" />
  </picture>
</a>
