#!/bin/sh
# In-VM driver (PID 1 / rdinit) for the vpnhide *KPM* QEMU harness.
#
# Unlike the .ko harness (init.sh), the KPM is already loaded at boot
# (embedded in the patched kernel image by KernelPatch), so there is no
# insmod. The control-v2 config is set at load time via embedded extra-args
# (`kptools -A "<snapshot>"`), so this driver is phase-agnostic: it fabricates a
# VPN-like `vpn0` interface and reports, for each detection vector, the count
# of `vpn0` hits as seen by a regular app UID. run-kpm.sh boots it twice — once
# with no target (the app must SEE vpn0) and once with uid 10000 targeted (the
# app must NOT see vpn0) — and diffs the counts.
set +e
export PATH=/usr/sbin:/usr/bin:/sbin:/bin

mount -t proc proc /proc 2>/dev/null
mount -t sysfs sys /sys 2>/dev/null
mount -t devtmpfs dev /dev 2>/dev/null

echo "##### VPNHIDE-KPM-TEST START #####"
echo "KREL=$(uname -r)"

# Did KernelPatch load our KPM and install hooks?
if dmesg | grep -q "KPM hooks installed"; then echo "KPMLOAD=ok"; else echo "KPMLOAD=FAIL"; fi
echo "KVER=$(dmesg | grep -oE 'kver=0x[0-9a-f]+' | head -1)"
dmesg | grep 'vpnhide:' | tail -20 | sed 's/^/KPMLOG=/'

# user-mode net so apk can fetch iproute2 (busybox ip can't add dummy devs)
ip link set eth0 up 2>/dev/null
ip addr add 10.0.2.15/24 dev eth0 2>/dev/null
ip route add default via 10.0.2.2 2>/dev/null
echo "nameserver 10.0.2.3" > /etc/resolv.conf
echo "https://dl-cdn.alpinelinux.org/alpine/v3.21/main" > /etc/apk/repositories
if apk add --no-cache iproute2 >/dev/null 2>&1; then echo "IPROUTE2=ok"; else echo "IPROUTE2=FAIL"; fi

# Hooks deliberately ignore Android system AIDs (< 10000). Run every shell
# vector as a regular app UID so this harness exercises the target path. The
# legacy Android kernels also require apps that open AF_INET sockets to carry
# the INTERNET permission's supplemental inet group (AID_INET = 3003).
TARGET_UID=10000
if addgroup -g 3003 android-inet >/dev/null 2>&1 &&
	adduser -D -u "$TARGET_UID" vpnhide-target >/dev/null 2>&1 &&
	addgroup vpnhide-target android-inet >/dev/null 2>&1; then
	echo "TARGET_USER=ok"
else
	echo "TARGET_USER=FAIL"
fi
run_as_target() {
	su -s /bin/sh vpnhide-target -c "$1"
}

# fabricate a VPN-like interface + routes through it (v4 + v6)
ip link add vpn0 type dummy 2>/dev/null
ip link set vpn0 up 2>/dev/null
ip addr add 10.9.0.1/24 dev vpn0 2>/dev/null
ip route add 10.9.9.0/24 dev vpn0 2>/dev/null
ip -6 addr add fd00:9::1/64 dev vpn0 2>/dev/null
ip -6 route add fd00:99::/64 dev vpn0 2>/dev/null
ip rule add uidrange "$TARGET_UID-$TARGET_UID" table 199 2>/dev/null

# The bind probe's actor runs as uid 10000 and issues a raw setsockopt syscall;
# a separate non-target observer inspects the same socket afterwards.  The host
# harness compares these raw fields across the notarget and target boots.
VPN0_IFINDEX=$(cat /sys/class/net/vpn0/ifindex 2>/dev/null)
if [ -x /bind-probe ] && [ -n "$VPN0_IFINDEX" ]; then
	/bind-probe vpn0 "$VPN0_IFINDEX" 2>/dev/null
fi

# By-name SIOCGIFHWADDR (dev_get_mac_address, 5.4+) / SIOCGIFADDR (devinet_ioctl)
# — these get-by-name ioctls do not go through dev_ifsioc_locked, and SIOCGIFADDR
# is a separate top-level path from the dev_ioctl dispatcher, so the `ifconfig
# vpn0` vector below (SIOCGIFFLAGS, dispatcher path) cannot isolate them. The
# probe drops to uid 10000 and prints HWADDR_ERRNO=/ADDR_ERRNO=; run-kpm.sh diffs
# them across the notarget/target boots.
if [ -x /iface-ioctl ]; then
	/iface-ioctl vpn0 2>/dev/null
fi

# Public /32 + /128 host-routes pinned to the physical uplink (eth0) — the routes
# a VPN client installs so tunnel packets reach the server. They leak the server's
# public IP through an RTM_GETROUTE dump even though vpn0 is hidden, so they must
# be hidden for a target the way the .ko hides them. eth0 is not a VPN iface, so
# this exercises the public-host-route path, not iface_is_vpn. The KPM now covers
# both v4 and v6 on every reference kernel in the CI matrix (5.x/6.x fib6_info
# plus the 4.14 rt6_info path), so
# no per-kver gate is needed here.
ip route add 1.2.3.4/32 dev eth0 2>/dev/null
ip -6 route add 2001:4860:4860::8888/128 dev eth0 2>/dev/null

# Vectors covered by the wired hooks. Count vpn0 hits as seen by the app, then
# count stable non-VPN entries so an over-trimmed empty dump fails loudly.
echo "VEC proc_route_v4=$(run_as_target 'grep -c vpn0 /proc/net/route 2>/dev/null')"        # fib_route_seq_show
echo "VEC keep_proc_route_v4=$(run_as_target "grep -c '^eth0' /proc/net/route 2>/dev/null")"
echo "VEC getifaddrs=$(run_as_target "ip addr show 2>/dev/null | grep -c 'vpn0'")"                # rtnl_fill_ifinfo
echo "VEC keep_getifaddrs=$(run_as_target "ip addr show 2>/dev/null | grep -c ': eth0:'")"
echo "VEC proc_route_v6=$(run_as_target 'grep -c vpn0 /proc/net/ipv6_route 2>/dev/null')"   # ipv6_route_seq_show
echo "VEC siocgifconf=$(run_as_target 'ifconfig -a 2>/dev/null | grep -c vpn0')"                  # sock_ioctl
echo "VEC keep_siocgifconf=$(run_as_target "ifconfig -a 2>/dev/null | grep -c '^eth0'")"
echo "VEC dev_ioctl=$(run_as_target 'ifconfig vpn0 2>/dev/null | grep -c vpn0')"                  # dev_ioctl
echo "VEC keep_dev_ioctl=$(run_as_target "ifconfig eth0 2>/dev/null | grep -c '^eth0'")"
echo "VEC netlink_route4=$(run_as_target 'ip route show table all 2>/dev/null | grep -c vpn0')"     # fib_dump_info (#86)
echo "VEC keep_netlink_route4=$(run_as_target "ip route show table all 2>/dev/null | grep -c 'dev eth0'")"
echo "VEC hostroute4=$(run_as_target "ip route show table all 2>/dev/null | grep -c '1\\.2\\.3\\.4'")" # fib_dump_info public host-route
echo "VEC hostroute6=$(run_as_target "ip -6 route show table all 2>/dev/null | grep -c '2001:4860'")" # rt6_fill_node public host-route
echo "VEC netlink_route6=$(run_as_target 'ip -6 route show table all 2>/dev/null | grep -c vpn0')"  # rt6_fill_node
echo "VEC policy_rule=$(run_as_target 'ip rule show 2>/dev/null | grep -c 199')"                    # fib_nl_fill_rule
echo "VEC keep_policy_rule=$(run_as_target "ip rule show 2>/dev/null | grep -c 'lookup main'")"
echo "VEC sysfs_stat=$(run_as_target 'test -e /sys/class/net/vpn0 && echo vpn0' | grep -c vpn0)"
echo "VEC sysfs_open=$(run_as_target 'cat /sys/class/net/vpn0/mtu 2>/dev/null && echo vpn0' | grep -c vpn0)"
echo "VEC sysfs_readdir=$(run_as_target 'ls /sys/class/net 2>/dev/null' | grep -c vpn0)"
echo "VEC keep_sysfs_readdir=$(run_as_target 'ls /sys/class/net 2>/dev/null' | grep -c eth0)"
echo "VEC proc_sys_stat=$(run_as_target 'test -e /proc/sys/net/ipv4/conf/vpn0 && echo vpn0' | grep -c vpn0)"
echo "VEC proc_sys_readdir=$(run_as_target 'ls /proc/sys/net/ipv4/conf 2>/dev/null' | grep -c vpn0)"
echo "VEC keep_proc_sys_readdir=$(run_as_target 'ls /proc/sys/net/ipv4/conf 2>/dev/null' | grep -c eth0)"
# Native getifaddrs() (RTM_GETLINK+RTM_GETADDR) — isolates inet*_fill_ifaddr.
if [ -x /gai ]; then
	GAI_OUT=$(run_as_target /gai 2>/dev/null)
	echo "VEC gai_getifaddrs=$(printf '%s\n' "$GAI_OUT" | sed -n 's/^GAI_VPN0=//p' | head -1)"
	echo "VEC keep_gai_getifaddrs=$(printf '%s\n' "$GAI_OUT" | sed -n 's/^GAI_OTHER=//p' | head -1)"
fi
if [ -x /ifconf ]; then
	run_as_target /ifconf 2>/dev/null
fi

PANIC_RE='Unable to handle|Internal error:|Oops|BUG:|Kernel panic'
PANIC=$(dmesg | grep -cE "$PANIC_RE")
echo "PANIC=$PANIC"
if [ "$PANIC" -ne 0 ]; then
	dmesg | grep -E "$PANIC_RE" | sed 's/^/PANICLOG=/'
fi
echo "##### VPNHIDE-KPM-TEST END #####"
poweroff -f
