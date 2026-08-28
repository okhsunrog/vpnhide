// AUTO-GENERATED from data/hooks.toml — do not edit by hand. Regenerate with: uv run scripts/codegen-hooks.py

#![allow(dead_code)]

/// Global hook id space (data/hooks.toml). bit N == hook id N.
#[repr(u32)]
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Hook {
    /// /proc/net/route — IPv4 route lines
    FibRouteSeqShow = 0,
    /// /proc/net/ipv6_route — IPv6 route lines
    Ipv6RouteSeqShow = 1,
    /// RTM_NEWLINK — getifaddrs() link enumeration
    RtnlFillIfinfo = 2,
    /// RTM_GETADDR — IPv4 address dump
    InetFillIfaddr = 3,
    /// RTM_GETADDR — IPv6 address dump
    Inet6FillIfaddr = 4,
    /// SIOCGIF* by name (per-interface ioctls)
    DevIoctl = 5,
    /// SIOCGIFCONF — interface list ioctl
    SockIoctl = 6,
    /// RTM_GETROUTE — IPv4 route dump (issue #86)
    FibDumpInfo = 7,
    /// RTM_GETROUTE — IPv6 route dump
    Rt6FillNode = 8,
    /// RTM_GETRULE — policy routing rules
    FibNlFillRule = 9,
    /// LinkProperties parcel/result sanitization
    LsposedLinkProperties = 10,
    /// NetworkCapabilities parcel/result sanitization
    LsposedNetworkCapabilities = 11,
    /// NetworkInfo parcel/result sanitization
    LsposedNetworkInfo = 12,
    /// Network handle replacement/filtering
    LsposedNetwork = 13,
    /// ConnectivityService synchronous result filtering
    LsposedConnectivityResult = 14,
    /// ConnectivityService callback filtering
    LsposedConnectivityCallback = 15,
    /// ConnectivityService Network handle APIs
    LsposedConnectivityNetwork = 16,
    /// PackageManager app-hiding filters
    LsposedPackageVisibility = 17,
    /// libc ioctl() SIOCGIF* interface probes
    ZygiskIoctl = 18,
    /// libc getifaddrs() interface enumeration
    ZygiskGetifaddrs = 19,
    /// openat() filtering for /proc/net routes and sockets
    ZygiskOpenat = 20,
    /// recvmsg() netlink dump filtering
    ZygiskRecvmsg = 21,
    /// recv() netlink dump filtering
    ZygiskRecv = 22,
    /// recvfrom() netlink dump filtering
    ZygiskRecvfrom = 23,
    /// __recvfrom_chk() fortified netlink dump filtering
    ZygiskRecvfromChk = 24,
    /// SO_BINDTODEVICE / SO_BINDTOIFINDEX pre-mutation denial
    SocketBindInterface = 25,
    /// libc setsockopt() best-effort socket-interface bind denial
    ZygiskSetsockopt = 26,
    /// Optional sysfs/proc-sys VPN interface path concealment
    FilesystemIfacePaths = 27,
}

impl Hook {
    /// This hook's bit in the control/stats wire mask.
    pub const fn bit(self) -> u32 {
        1u32 << self as u32
    }

    /// This hook's canonical config name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::FibRouteSeqShow => "fib_route_seq_show",
            Self::Ipv6RouteSeqShow => "ipv6_route_seq_show",
            Self::RtnlFillIfinfo => "rtnl_fill_ifinfo",
            Self::InetFillIfaddr => "inet_fill_ifaddr",
            Self::Inet6FillIfaddr => "inet6_fill_ifaddr",
            Self::DevIoctl => "dev_ioctl",
            Self::SockIoctl => "sock_ioctl",
            Self::FibDumpInfo => "fib_dump_info",
            Self::Rt6FillNode => "rt6_fill_node",
            Self::FibNlFillRule => "fib_nl_fill_rule",
            Self::LsposedLinkProperties => "lsposed_link_properties",
            Self::LsposedNetworkCapabilities => "lsposed_network_capabilities",
            Self::LsposedNetworkInfo => "lsposed_network_info",
            Self::LsposedNetwork => "lsposed_network",
            Self::LsposedConnectivityResult => "lsposed_connectivity_result",
            Self::LsposedConnectivityCallback => "lsposed_connectivity_callback",
            Self::LsposedConnectivityNetwork => "lsposed_connectivity_network",
            Self::LsposedPackageVisibility => "lsposed_package_visibility",
            Self::ZygiskIoctl => "zygisk_ioctl",
            Self::ZygiskGetifaddrs => "zygisk_getifaddrs",
            Self::ZygiskOpenat => "zygisk_openat",
            Self::ZygiskRecvmsg => "zygisk_recvmsg",
            Self::ZygiskRecv => "zygisk_recv",
            Self::ZygiskRecvfrom => "zygisk_recvfrom",
            Self::ZygiskRecvfromChk => "zygisk_recvfrom_chk",
            Self::SocketBindInterface => "socket_bind_interface",
            Self::ZygiskSetsockopt => "zygisk_setsockopt",
            Self::FilesystemIfacePaths => "filesystem_iface_paths",
        }
    }

    /// Resolve a canonical config hook name.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "fib_route_seq_show" => Some(Self::FibRouteSeqShow),
            "ipv6_route_seq_show" => Some(Self::Ipv6RouteSeqShow),
            "rtnl_fill_ifinfo" => Some(Self::RtnlFillIfinfo),
            "inet_fill_ifaddr" => Some(Self::InetFillIfaddr),
            "inet6_fill_ifaddr" => Some(Self::Inet6FillIfaddr),
            "dev_ioctl" => Some(Self::DevIoctl),
            "sock_ioctl" => Some(Self::SockIoctl),
            "fib_dump_info" => Some(Self::FibDumpInfo),
            "rt6_fill_node" => Some(Self::Rt6FillNode),
            "fib_nl_fill_rule" => Some(Self::FibNlFillRule),
            "lsposed_link_properties" => Some(Self::LsposedLinkProperties),
            "lsposed_network_capabilities" => Some(Self::LsposedNetworkCapabilities),
            "lsposed_network_info" => Some(Self::LsposedNetworkInfo),
            "lsposed_network" => Some(Self::LsposedNetwork),
            "lsposed_connectivity_result" => Some(Self::LsposedConnectivityResult),
            "lsposed_connectivity_callback" => Some(Self::LsposedConnectivityCallback),
            "lsposed_connectivity_network" => Some(Self::LsposedConnectivityNetwork),
            "lsposed_package_visibility" => Some(Self::LsposedPackageVisibility),
            "zygisk_ioctl" => Some(Self::ZygiskIoctl),
            "zygisk_getifaddrs" => Some(Self::ZygiskGetifaddrs),
            "zygisk_openat" => Some(Self::ZygiskOpenat),
            "zygisk_recvmsg" => Some(Self::ZygiskRecvmsg),
            "zygisk_recv" => Some(Self::ZygiskRecv),
            "zygisk_recvfrom" => Some(Self::ZygiskRecvfrom),
            "zygisk_recvfrom_chk" => Some(Self::ZygiskRecvfromChk),
            "socket_bind_interface" => Some(Self::SocketBindInterface),
            "zygisk_setsockopt" => Some(Self::ZygiskSetsockopt),
            "filesystem_iface_paths" => Some(Self::FilesystemIfacePaths),
            _ => None,
        }
    }
}

pub const HOOK_COUNT: u32 = 28;

/// Hooks owned by each backend: apply `mask & own`.
pub const KERNEL_HOOK_MASK: u32 = 0x20003ff;
pub const KMOD_HOOK_MASK: u32 = 0x8000000;
pub const KPM_HOOK_MASK: u32 = 0x8000000;
pub const KPATCH_HOOK_MASK: u32 = 0x8000000;
pub const ZYGISK_HOOK_MASK: u32 = 0xdfc0000;
pub const LSPOSED_HOOK_MASK: u32 = 0x3fc00;

/// status error codes (protocol §5.1).
#[repr(u32)]
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum StatusError {
    /// healthy; every requested, owned hook installed
    Ok = 0,
    /// no offset table for the running kernel — refused, no hooks
    UnsupportedKver = 1,
    /// the other kernel backend (.ko<->KPM) is loaded — refused (docs/storage.md §4.3)
    ConflictingBackend = 2,
    /// a required kallsyms symbol was missing — refused
    SymbolResolutionFailed = 3,
    /// installed, but some owned hooks did not resolve (see the hooks mask)
    PartialHooks = 4,
}

/// backend ids (protocol §4.3 `status backend <id>`).
#[repr(u32)]
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Backend {
    /// .ko kretprobe backend
    Kmod = 0,
    /// KernelPatch Module backend
    Kpm = 1,
    /// Zygisk libc-hook backend
    Zygisk = 2,
    /// LSPosed Java-hook backend (system_server)
    Lsposed = 3,
    /// in-tree source-patched kernel backend (CONFIG_VPNHIDE=y); same kernel hooks as .ko, no kprobes
    Kpatch = 4,
}
