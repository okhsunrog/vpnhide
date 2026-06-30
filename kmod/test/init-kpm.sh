#!/bin/sh
# In-VM driver (PID 1 / rdinit) for the vpnhide *KPM* QEMU harness.
#
# Unlike the .ko harness (init.sh), the KPM is already loaded at boot
# (embedded in the patched kernel image by KernelPatch), so there is no
# insmod. Target UIDs are set at load time via the embedded extra-args
# (`kptools -A "<uids>"`), so this driver is phase-agnostic: it fabricates a
# VPN-like `vpn0` interface and reports, for each detection vector, the count
# of `vpn0` hits as seen by *this* (root, uid 0) process. run-kpm.sh boots it
# twice — once with no target (root must SEE vpn0) and once with target=0
# (root must NOT see vpn0) — and diffs the counts.
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

# user-mode net so apk can fetch iproute2 (busybox ip can't add dummy devs)
ip link set eth0 up 2>/dev/null
ip addr add 10.0.2.15/24 dev eth0 2>/dev/null
ip route add default via 10.0.2.2 2>/dev/null
echo "nameserver 10.0.2.3" > /etc/resolv.conf
echo "https://dl-cdn.alpinelinux.org/alpine/v3.21/main" > /etc/apk/repositories
if apk add --no-cache iproute2 >/dev/null 2>&1; then echo "IPROUTE2=ok"; else echo "IPROUTE2=FAIL"; fi

# fabricate a VPN-like interface + routes through it (v4 + v6)
ip link add vpn0 type dummy 2>/dev/null
ip link set vpn0 up 2>/dev/null
ip addr add 10.9.0.1/24 dev vpn0 2>/dev/null
ip route add 10.9.9.0/24 dev vpn0 2>/dev/null
ip -6 addr add fd00:9::1/64 dev vpn0 2>/dev/null
ip -6 route add fd00:99::/64 dev vpn0 2>/dev/null
ip rule add uidrange 0-0 table 199 2>/dev/null

# Public /32 host-route pinned to the physical uplink (eth0) — the route a VPN
# client installs so tunnel packets reach the server. It leaks the server's
# public IP through an RTM_GETROUTE dump even though vpn0 is hidden, so it must
# be hidden for a target the same way the .ko hides it. eth0 is not a VPN iface,
# so this exercises the public-host-route path, not iface_is_vpn.
ip route add 1.2.3.4/32 dev eth0 2>/dev/null

# Vectors covered by the wired hooks. Count vpn0 hits as seen by root, then
# count stable non-VPN entries so an over-trimmed empty dump fails loudly.
echo "VEC proc_route_v4=$(grep -c vpn0 /proc/net/route 2>/dev/null)"        # fib_route_seq_show
echo "VEC keep_proc_route_v4=$(grep -c '^eth0' /proc/net/route 2>/dev/null)"
echo "VEC getifaddrs=$(ip addr show 2>/dev/null | grep -c 'vpn0')"                # rtnl_fill_ifinfo
echo "VEC keep_getifaddrs=$(ip addr show 2>/dev/null | grep -c ': eth0:')"
echo "VEC proc_route_v6=$(grep -c vpn0 /proc/net/ipv6_route 2>/dev/null)"   # ipv6_route_seq_show
echo "VEC siocgifconf=$(ifconfig -a 2>/dev/null | grep -c vpn0)"                  # sock_ioctl
echo "VEC keep_siocgifconf=$(ifconfig -a 2>/dev/null | grep -c '^eth0')"
echo "VEC dev_ioctl=$(ifconfig vpn0 2>/dev/null | grep -c vpn0)"                  # dev_ioctl
echo "VEC keep_dev_ioctl=$(ifconfig eth0 2>/dev/null | grep -c '^eth0')"
echo "VEC netlink_route4=$(ip route show table all 2>/dev/null | grep -c vpn0)"     # fib_dump_info (#86)
echo "VEC keep_netlink_route4=$(ip route show table all 2>/dev/null | grep -c 'dev eth0')"
echo "VEC hostroute4=$(ip route show table all 2>/dev/null | grep -c '1\.2\.3\.4')" # fib_dump_info public host-route
echo "VEC netlink_route6=$(ip -6 route show table all 2>/dev/null | grep -c vpn0)"  # rt6_fill_node
echo "VEC policy_rule=$(ip rule show 2>/dev/null | grep -c 199)"                    # fib_nl_fill_rule
echo "VEC keep_policy_rule=$(ip rule show 2>/dev/null | grep -c 'lookup main')"
# Native getifaddrs() (RTM_GETLINK+RTM_GETADDR) — isolates inet*_fill_ifaddr.
if [ -x /gai ]; then
	GAI_OUT=$(/gai 2>/dev/null)
	echo "VEC gai_getifaddrs=$(printf '%s\n' "$GAI_OUT" | sed -n 's/^GAI_VPN0=//p' | head -1)"
	echo "VEC keep_gai_getifaddrs=$(printf '%s\n' "$GAI_OUT" | sed -n 's/^GAI_OTHER=//p' | head -1)"
fi

PANIC=$(dmesg | grep -ci 'Unable to handle\|Internal error\|Oops\|BUG:\|Kernel panic')
echo "PANIC=$PANIC"
echo "##### VPNHIDE-KPM-TEST END #####"
poweroff -f
