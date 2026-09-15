# 检测向量与覆盖

应用有好几种方式察觉 VPN。每一种都对应 VPN Hide 的一个角色或后端。这是面向用户的
概览；按每个后端细分的完整矩阵在仓库的开发者文档里。

| 应用能检查什么 | 由谁覆盖 |
|---|---|
| `NetworkCapabilities`、`NetworkInfo`、`LinkProperties`（Java API） | Java（LSPosed） |
| 网络接口列表（`getifaddrs` / `NetworkInterface`） | Native（内核 / libc） |
| 路由规则与 netlink 转储 | Native |
| 绑定到隧道接口的套接字 | Native |
| 已安装的 VPN 应用（软件包扫描） | Apps |
| 本地回环的 VPN / 代理端口 | Ports |

## 说明

- **Java 与 Native。** 有些信号既通过 Java API、也通过 libc/内核调用到达应用。Java
  覆盖前者，原生后端覆盖后者——两个都开最稳妥。
- **直接原始系统调用。** 绕过 libc、直接调用内核（`svc #0`）的应用，只能由**内核
  后端**（kmod / 内置 / KPM）覆盖，Zygisk 的 libc 钩子覆盖不到。
- **SELinux。** 在某些 ROM 上，某个向量已被系统的 SELinux 策略挡下——被隐藏了，但不是
  VPN Hide 做的，因此因设备而异。
