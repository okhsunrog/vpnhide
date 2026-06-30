#!/bin/sh
# In-VM test driver (PID 1 / rdinit) for the vpnhide kmod QEMU harness.
#
# Boots inside a minimal Alpine initramfs, loads /vpnhide_kmod.ko, fabricates
# a VPN-like interface (`vpn0`, a dummy netdev whose name matches the VPN
# prefix list), then exercises every detection vector twice — once as a
# non-target UID (must still see vpn0) and once as a target UID (must NOT
# see vpn0). Emits machine-parseable `RESULT <vector>=PASS|FAIL` lines plus a
# final `SUMMARY` line, then powers off so QEMU exits.
#
# Output is consumed by run.sh on the host via the serial console.
set +e
export PATH=/usr/sbin:/usr/bin:/sbin:/bin

mount -t proc proc /proc 2>/dev/null
mount -t sysfs sys /sys 2>/dev/null
mount -t devtmpfs dev /dev 2>/dev/null

echo "##### VPNHIDE-QEMU-TEST START #####"
echo "KREL=$(uname -r)"

# --- bring up user-mode networking so apk can fetch iproute2 ----------------
ip link set eth0 up 2>/dev/null
ip addr add 10.0.2.15/24 dev eth0 2>/dev/null
ip route add default via 10.0.2.2 2>/dev/null
echo "nameserver 10.0.2.3" > /etc/resolv.conf
echo "https://dl-cdn.alpinelinux.org/alpine/v3.21/main" > /etc/apk/repositories
if apk add --no-cache iproute2 >/dev/null 2>&1; then echo "IPROUTE2=ok"; else echo "IPROUTE2=FAIL"; fi

# --- load the module --------------------------------------------------------
if insmod /vpnhide_kmod.ko; then echo "INSMOD=ok"; else echo "INSMOD=FAIL"; fi
REGISTERED=$(dmesg | grep -c 'vpnhide:.*registered')
echo "REGISTERED=$REGISTERED"

# Write a control-protocol config snapshot (docs/protocol.md) enabling every
# kernel hook (mask 0x3ff) for a single UID, with debug logging on. Replaces
# the old `echo <uid> > /proc/vpnhide_targets` + `echo 1 > /proc/vpnhide_debug`
# — both folded into the one /proc/vpnhide_ctl node.
set_target() {
	printf 'vpnhide 1 config\ndebug 1\ntarget 0x%x 0x3ff\n' "$1" \
		> /proc/vpnhide_ctl 2>/dev/null
}

# --- fabricate a VPN-like interface + routes + a per-uid policy rule ---------
ip link add vpn0 type dummy 2>/dev/null
ip link set vpn0 up 2>/dev/null
ip addr add 10.9.0.1/24 dev vpn0 2>/dev/null
ip route add 10.9.9.0/24 dev vpn0 2>/dev/null
ip -6 addr add fd00:9::1/64 dev vpn0 2>/dev/null
ip -6 route add fd00:99::/64 dev vpn0 2>/dev/null
ip rule add uidrange 0-0 table 199 2>/dev/null

# Public /32 host-route pinned to the physical uplink (eth0) — the route a VPN
# client installs so tunnel packets reach the server. eth0 is not a VPN iface,
# so hiding it from a target exercises is_public_host_route_via_physical, not
# the VPN-name matcher.
ip route add 1.2.3.4/32 dev eth0 2>/dev/null
# IPv6 analogue: a public /128 host-route pinned to the physical uplink.
ip -6 route add 2001:4860:4860::8888/128 dev eth0 2>/dev/null

PASS=0
FAIL=0

# check_hide <name> <shell-command> <grep-pattern>
# Asserts: non-target (uid 5555) sees the pattern, target (uid 0) does not.
check_hide() {
	_name=$1
	_cmd=$2
	_pat=$3
	set_target 5555
	_nt=$(eval "$_cmd" 2>/dev/null | grep -c -- "$_pat")
	set_target 0
	_tg=$(eval "$_cmd" 2>/dev/null | grep -c -- "$_pat")
	if [ "$_nt" -gt 0 ] && [ "$_tg" -eq 0 ]; then
		echo "RESULT $_name=PASS (nontarget=$_nt target=$_tg)"
		PASS=$((PASS + 1))
	else
		echo "RESULT $_name=FAIL (nontarget=$_nt target=$_tg)"
		FAIL=$((FAIL + 1))
	fi
}

check_keep() {
	_name=$1
	_cmd=$2
	_pat=$3
	set_target 5555
	_nt=$(eval "$_cmd" 2>/dev/null | grep -c -- "$_pat")
	set_target 0
	_tg=$(eval "$_cmd" 2>/dev/null | grep -c -- "$_pat")
	if [ "$_nt" -gt 0 ] && [ "$_tg" -gt 0 ]; then
		echo "RESULT $_name=PASS (nontarget=$_nt target=$_tg mode=nonvpn)"
		PASS=$((PASS + 1))
	else
		echo "RESULT $_name=FAIL (nontarget=$_nt target=$_tg mode=nonvpn)"
		FAIL=$((FAIL + 1))
	fi
}

check_keep_exact() {
	_name=$1
	_cmd=$2
	_pat=$3
	set_target 5555
	_nt=$(eval "$_cmd" 2>/dev/null | grep -c -- "$_pat")
	set_target 0
	_tg=$(eval "$_cmd" 2>/dev/null | grep -c -- "$_pat")
	if [ "$_nt" -gt 0 ] && [ "$_tg" -eq "$_nt" ]; then
		echo "RESULT $_name=PASS (nontarget=$_nt target=$_tg mode=exact)"
		PASS=$((PASS + 1))
	else
		echo "RESULT $_name=FAIL (nontarget=$_nt target=$_tg mode=exact)"
		FAIL=$((FAIL + 1))
	fi
}

gai_field() {
	/gai 2>/dev/null | sed -n "s/^$1=//p" | head -1
}

check_gai() {
	if [ ! -x /gai ]; then
		echo "RESULT gai_getifaddrs=SKIP (no bionic getifaddrs probe available)"
		echo "RESULT keep_gai_getifaddrs=SKIP (no bionic getifaddrs probe available)"
		return
	fi

	set_target 5555
	_nt_vpn=$(gai_field GAI_VPN0)
	_nt_other=$(gai_field GAI_OTHER)
	set_target 0
	_tg_vpn=$(gai_field GAI_VPN0)
	_tg_other=$(gai_field GAI_OTHER)
	[ -n "$_nt_vpn" ] || _nt_vpn=-1
	[ -n "$_nt_other" ] || _nt_other=-1
	[ -n "$_tg_vpn" ] || _tg_vpn=-1
	[ -n "$_tg_other" ] || _tg_other=-1

	if [ "$_nt_vpn" -gt 0 ] && [ "$_tg_vpn" -eq 0 ]; then
		echo "RESULT gai_getifaddrs=PASS (nontarget=$_nt_vpn target=$_tg_vpn)"
		PASS=$((PASS + 1))
	else
		echo "RESULT gai_getifaddrs=FAIL (nontarget=$_nt_vpn target=$_tg_vpn)"
		FAIL=$((FAIL + 1))
	fi

	if [ "$_nt_other" -gt 0 ] && [ "$_tg_other" -gt 0 ]; then
		echo "RESULT keep_gai_getifaddrs=PASS (nontarget=$_nt_other target=$_tg_other mode=nonvpn)"
		PASS=$((PASS + 1))
	else
		echo "RESULT keep_gai_getifaddrs=FAIL (nontarget=$_nt_other target=$_tg_other mode=nonvpn)"
		FAIL=$((FAIL + 1))
	fi
}

# vector -> hook it exercises
check_hide getifaddrs      "ip addr show"                 "vpn0"   # rtnl_fill_ifinfo + inet*_fill_ifaddr
check_hide siocgifconf     "ifconfig -a"                  "vpn0"   # sock_ioctl
check_hide dev_ioctl       "ifconfig vpn0"                "vpn0"   # dev_ioctl
check_hide proc_route_v4   "cat /proc/net/route"          "vpn0"   # fib_route_seq_show
check_hide proc_route_v6   "cat /proc/net/ipv6_route"     "vpn0"   # ipv6_route_seq_show
check_hide netlink_route4  "ip route show table all"      "vpn0"   # fib_dump_info
check_hide hostroute4      "ip route show table all"      "1\.2\.3\.4" # fib_dump_info public host-route
check_hide netlink_route6  "ip -6 route show table all"   "vpn0"   # rt6_fill_node
check_hide hostroute6      "ip -6 route show table all"   "2001:4860" # rt6_fill_node public host-route
check_hide policy_rule     "ip rule show"                 "199"    # fib_nl_fill_rule
check_gai

# Non-VPN entries must survive target filtering. This catches over-trimming
# regressions where a hook hides the whole dump instead of only vpn0 rows.
check_keep_exact keep_proc_route_v4   "cat /proc/net/route"     "^eth0"
check_keep_exact keep_getifaddrs      "ip addr show"            ": eth0:"
check_keep_exact keep_siocgifconf     "ifconfig -a"             "^eth0"
check_keep_exact keep_dev_ioctl       "ifconfig eth0"           "^eth0"
check_keep keep_netlink_route4  "ip route show table all" "dev eth0"
check_keep_exact keep_policy_rule     "ip rule show"            "lookup main"

PANIC=$(dmesg | grep -ci 'Unable to handle\|Internal error\|Oops\|BUG:\|Kernel panic')
echo "PANIC=$PANIC"
echo "SUMMARY pass=$PASS fail=$FAIL panic=$PANIC registered=$REGISTERED"
echo "##### VPNHIDE-QEMU-TEST END #####"

poweroff -f
