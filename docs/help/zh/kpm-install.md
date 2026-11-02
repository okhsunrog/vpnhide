# 安装 KPM 后端

KPM 是完整 Core ZIP 中的内核后端，需要独立的 KernelPatch 运行时。安装
[配套 APK 和 Core](first-install.md)。

## 它需要 KernelPatch 运行时

Core 必须确认运行时适用并完成认证。root 管理器的名称或某个目录的存在本身并不能
证明就绪。Core 不包含也不刷入 KernelPatch engine，运行时部署是独立安装操作。

## 步骤

1. 确认内核和运行时受支持且已就绪。
2. 安装完整 Core ZIP 和配套 APK。
3. 仅在 Core 请求且 trusted-token 访问不可用时提供 SuperKey。
4. 按要求重启、解锁并查看 Core 结果。

Core 负责加载和配置 KPM。不要提取并手动加载内部 payload，也不要在 Core 旁另装
独立 VPN Hide KPM ZIP。

## SuperKey 存储

认证需要 APatch/FolkPatch SuperKey 时，使用**设置 → 安全**。保存的副本仅供 root
访问；清除操作移除该副本。不要把密钥加入报告。见
[安全与隐私](security-and-privacy.md#保存的密钥)。

## 受支持的内核

支持取决于经过验证的内核布局和实际运行时，而不只是版本标签。兼容性未知时会保持
未验证。Core 根据设备判断适用性，不会假定所有非 GKI 内核都能工作。

## 为什么某些内核上显示 10 个钩子

已安装钩子的数量表示配置情况，不等于完整诊断覆盖。不同内核提供的路径不同。
即使部分钩子已安装，缺失或未验证的必需检查仍会显示。

## 如果已安装但未生效

检查运行时认证、内核适用性及 Core 的等待/错误状态。重复安装相同 payload 无法修复
不受支持的偏移。见[模块未生效](module-not-active.md)或[收集报告](collect-report.md)。

## 下载并安装 KPatch-Next-Module

KernelPatch 部署独立于 VPN Hide 通用安装包。使用适用于实际 root/内核组合的受支持
安装流程，并保留可恢复的原始 boot image。包名本身不能证明兼容性。Core 中的 KPM
payload 不是 engine 安装器；不要为了修复 Core 状态错误套用旧的未验证 boot 刷写步骤。
