#!/bin/sh
# In-VM test driver (PID 1 / rdinit) for the vpnhide kmod QEMU harness.
#
# Boots inside a minimal Alpine initramfs, loads /vpnhide_kmod.ko, fabricates
# a VPN-like interface (`vpn0`, a dummy netdev whose name matches the VPN
# prefix list), then exercises the available detection vectors twice — once as
# a non-target UID (must still see vpn0) and once as a target UID (must NOT see
# vpn0). Optional native probes emit SKIP when not supplied. The driver emits
# machine-parseable `RESULT <vector>=PASS|FAIL|SKIP` lines plus a final `SUMMARY`
# line, then powers off so QEMU exits.
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

# Hooks deliberately ignore Android system AIDs (< 10000). Run every shell
# vector as a regular app UID so this harness exercises the target path. The
# legacy Android kernels also require apps that open AF_INET sockets to carry
# the INTERNET permission's supplemental inet group (AID_INET = 3003).
TARGET_UID=10000
NONMATCH_UID=10002
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

# --- load / verify the backend ----------------------------------------------
# Which backend this boot exercises. The host env does not cross into the VM, so
# builtin/test/run.sh drops a /vpnhide_backend marker into the initramfs; default
# is the .ko harness.
VPNHIDE_TEST_BACKEND="${VPNHIDE_TEST_BACKEND:-$([ -f /vpnhide_backend ] && cat /vpnhide_backend 2>/dev/null || echo kmod)}"
# kmod: insmod the .ko and count the kretprobe registrations from dmesg.
# builtin: nothing to insmod — the driver is compiled into the kernel
# (CONFIG_VPNHIDE=y); "loaded" == the control node exists, and every owned hook
# is a compiled-in call site so there is no per-hook registration to count.
if [ "${VPNHIDE_TEST_BACKEND:-kmod}" = "builtin" ]; then
	if [ -e /proc/vpnhide_ctl ]; then echo "INSMOD=ok (builtin, in-tree)"; else echo "INSMOD=FAIL"; fi
	REGISTERED=$([ -e /proc/vpnhide_ctl ] && echo 1 || echo 0)
	echo "REGISTERED=$REGISTERED"
else
	if insmod /vpnhide_kmod.ko filesystem_hiding=1; then echo "INSMOD=ok"; else echo "INSMOD=FAIL"; fi
	REGISTERED=$(dmesg | grep -c 'vpnhide:.*registered')
	echo "REGISTERED=$REGISTERED"
fi

# Write a control-protocol config snapshot (docs/protocol.md) enabling every
# kernel and optional filesystem hooks (mask 0xa0003ff) for a single UID, with
# debug logging on. Replaces
# the old `echo <uid> > /proc/vpnhide_targets` + `echo 1 > /proc/vpnhide_debug`
# — both folded into the one /proc/vpnhide_ctl node.
set_target() {
	printf 'vpnhide 2 config\ndebug 1\ntargets a0003ff %x\nend 1\n' "$1" \
		> /proc/vpnhide_ctl 2>/dev/null
}

# --- fabricate a VPN-like interface + routes + a per-uid policy rule ---------
ip link add vpn0 type dummy 2>/dev/null
ip link set vpn0 up 2>/dev/null
ip addr add 10.9.0.1/24 dev vpn0 2>/dev/null
ip route add 10.9.9.0/24 dev vpn0 2>/dev/null
ip -6 addr add fd00:9::1/64 dev vpn0 2>/dev/null
ip -6 route add fd00:99::/64 dev vpn0 2>/dev/null
ip rule add uidrange "$TARGET_UID-$TARGET_UID" table 199 2>/dev/null

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
# Asserts: app uid 10000 sees the pattern when another UID is targeted, then
# stops seeing it when uid 10000 is targeted.
check_hide() {
	_name=$1
	_cmd=$2
	_pat=$3
	set_target "$NONMATCH_UID"
	_nt=$(run_as_target "$_cmd" 2>/dev/null | grep -c -- "$_pat")
	set_target "$TARGET_UID"
	_tg=$(run_as_target "$_cmd" 2>/dev/null | grep -c -- "$_pat")
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
	set_target "$NONMATCH_UID"
	_nt=$(run_as_target "$_cmd" 2>/dev/null | grep -c -- "$_pat")
	set_target "$TARGET_UID"
	_tg=$(run_as_target "$_cmd" 2>/dev/null | grep -c -- "$_pat")
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
	set_target "$NONMATCH_UID"
	_nt=$(run_as_target "$_cmd" 2>/dev/null | grep -c -- "$_pat")
	set_target "$TARGET_UID"
	_tg=$(run_as_target "$_cmd" 2>/dev/null | grep -c -- "$_pat")
	if [ "$_nt" -gt 0 ] && [ "$_tg" -eq "$_nt" ]; then
		echo "RESULT $_name=PASS (nontarget=$_nt target=$_tg mode=exact)"
		PASS=$((PASS + 1))
	else
		echo "RESULT $_name=FAIL (nontarget=$_nt target=$_tg mode=exact)"
		FAIL=$((FAIL + 1))
	fi
}

gai_field() {
	run_as_target /gai 2>/dev/null | sed -n "s/^$1=//p" | head -1
}

check_gai() {
	if [ ! -x /gai ]; then
		echo "RESULT gai_getifaddrs=SKIP (no bionic getifaddrs probe available)"
		echo "RESULT keep_gai_getifaddrs=SKIP (no bionic getifaddrs probe available)"
		return
	fi

	set_target "$NONMATCH_UID"
	_nt_vpn=$(gai_field GAI_VPN0)
	_nt_other=$(gai_field GAI_OTHER)
	set_target "$TARGET_UID"
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

ifc_field() {
	run_as_target /ifconf 2>/dev/null | sed -n "s/^$1=//p" | head -1
}

# SIOCGIFCONF probe: the target's size query must agree with its filtered fill,
# drop below the non-target size, and leave no stale names past ifc_len.
check_ifconf() {
	if [ ! -x /ifconf ]; then
		echo "RESULT ifconf_probe=SKIP (no ifconf probe available)"
		return
	fi

	set_target "$NONMATCH_UID"
	_nt_size=$(ifc_field IFCONF_SIZE)
	_nt_tail=$(ifc_field IFCONF_TAIL)
	set_target "$TARGET_UID"
	_tg_size=$(ifc_field IFCONF_SIZE)
	_tg_fill=$(ifc_field IFCONF_FILL)
	_tg_vpn=$(ifc_field IFCONF_FILL_VPN)
	_tg_tail=$(ifc_field IFCONF_TAIL)
	[ -n "$_nt_size" ] || _nt_size=-1
	[ -n "$_tg_size" ] || _tg_size=-1
	[ -n "$_tg_fill" ] || _tg_fill=-1
	[ -n "$_tg_vpn" ] || _tg_vpn=-1
	[ -n "$_nt_tail" ] || _nt_tail=-1
	[ -n "$_tg_tail" ] || _tg_tail=-1

	# tg_vpn==0 guards against matching two equally-broken numbers: the fill
	# must actually be vpn-free for the size==fill agreement to mean anything.
	if [ "$_tg_vpn" -eq 0 ] && [ "$_nt_tail" -eq 0 ] &&
		[ "$_tg_tail" -eq 0 ] && [ "$_tg_size" -eq "$_tg_fill" ] &&
		[ "$_nt_size" -gt "$_tg_size" ]; then
		echo "RESULT ifconf_probe=PASS (nt_size=$_nt_size tg_size=$_tg_size tg_fill=$_tg_fill nt_tail=$_nt_tail tg_tail=$_tg_tail)"
		PASS=$((PASS + 1))
	else
		echo "RESULT ifconf_probe=FAIL (nt_size=$_nt_size tg_size=$_tg_size tg_fill=$_tg_fill tg_vpn=$_tg_vpn nt_tail=$_nt_tail tg_tail=$_tg_tail)"
		FAIL=$((FAIL + 1))
	fi
}

bind_field() {
	_output=$1
	_key=$2
	printf '%s\n' "$_output" | sed -n "s/^${_key}=//p" | head -1
}

# A return code is not enough for a bind-denial test: a return-only kretprobe
# can report ENODEV after the kernel has already set sk_bound_dev_if.  The
# native probe has a target child perform a raw syscall and a non-target child
# inspect the same socket afterwards.
check_socket_bind() {
	if [ ! -x /bind-probe ]; then
		for _vec in bind_device_raw bind_device_nul bind_bad_pointer bind_bad_length bind_ifindex keep_bind_device bind_absent_name; do
			echo "RESULT $_vec=SKIP (no socket bind probe available)"
		done
		return
	fi

	# Root reads the fixture metadata before becoming a target itself; the VFS
	# feature must not hide vpn0 from this non-target setup step.
	set_target 5555
	_vpn_ifindex=$(cat /sys/class/net/vpn0/ifindex 2>/dev/null)
	if [ -z "$_vpn_ifindex" ]; then
		for _vec in bind_device_raw bind_device_nul bind_bad_pointer bind_bad_length bind_ifindex keep_bind_device bind_absent_name; do
			echo "RESULT $_vec=FAIL (vpn0 ifindex unavailable)"
			FAIL=$((FAIL + 1))
		done
		return
	fi

	# The probe actor is uid 10000. First make it a non-target, then target it.
	set_target "$NONMATCH_UID"
	_nt=$(/bind-probe vpn0 "$_vpn_ifindex" 2>/dev/null)
	set_target "$TARGET_UID"
	_tg=$(/bind-probe vpn0 "$_vpn_ifindex" 2>/dev/null)

	for _case in \
		"bind_device_raw BIND_NAME_RAW" \
		"bind_device_nul BIND_NAME_NUL" \
		"bind_ifindex BIND_INDEX"; do
		_vec=${_case%% *}
		_prefix=${_case#* }
		_nt_errno=$(bind_field "$_nt" "${_prefix}_ERRNO")
		_nt_state=$(bind_field "$_nt" "${_prefix}_STATE")
		_tg_errno=$(bind_field "$_tg" "${_prefix}_ERRNO")
		_tg_state=$(bind_field "$_tg" "${_prefix}_STATE")
		[ -n "$_nt_errno" ] || _nt_errno=-1
		[ -n "$_nt_state" ] || _nt_state=-1
		[ -n "$_tg_errno" ] || _tg_errno=-1
		[ -n "$_tg_state" ] || _tg_state=-1
		if [ "$_nt_errno" -eq 0 ] && [ "$_nt_state" -eq 1 ] && \
			[ "$_tg_errno" -eq 19 ] && [ "$_tg_state" -eq 0 ]; then
			echo "RESULT $_vec=PASS (notarget=bound target=ENODEV+unbound)"
			PASS=$((PASS + 1))
		else
			echo "RESULT $_vec=FAIL (nt_errno=$_nt_errno nt_state=$_nt_state tg_errno=$_tg_errno tg_state=$_tg_state)"
			FAIL=$((FAIL + 1))
		fi
	done

	_nt_errno=$(bind_field "$_nt" BIND_BADPTR_ERRNO)
	_nt_state=$(bind_field "$_nt" BIND_BADPTR_STATE)
	_tg_errno=$(bind_field "$_tg" BIND_BADPTR_ERRNO)
	_tg_state=$(bind_field "$_tg" BIND_BADPTR_STATE)
	[ -n "$_nt_errno" ] || _nt_errno=-1
	[ -n "$_nt_state" ] || _nt_state=-1
	[ -n "$_tg_errno" ] || _tg_errno=-1
	[ -n "$_tg_state" ] || _tg_state=-1
	if [ "$_nt_errno" -eq 14 ] && [ "$_nt_state" -eq 0 ] && \
		[ "$_tg_errno" -eq 14 ] && [ "$_tg_state" -eq 0 ]; then
		echo "RESULT bind_bad_pointer=PASS (EFAULT+unbound)"
		PASS=$((PASS + 1))
	else
		echo "RESULT bind_bad_pointer=FAIL (nt_errno=$_nt_errno nt_state=$_nt_state tg_errno=$_tg_errno tg_state=$_tg_state)"
		FAIL=$((FAIL + 1))
	fi

	_nt_errno=$(bind_field "$_nt" BIND_BADLEN_ERRNO)
	_nt_state=$(bind_field "$_nt" BIND_BADLEN_STATE)
	_tg_errno=$(bind_field "$_tg" BIND_BADLEN_ERRNO)
	_tg_state=$(bind_field "$_tg" BIND_BADLEN_STATE)
	[ -n "$_nt_errno" ] || _nt_errno=-1
	[ -n "$_nt_state" ] || _nt_state=-1
	[ -n "$_tg_errno" ] || _tg_errno=-1
	[ -n "$_tg_state" ] || _tg_state=-1
	if [ "$_nt_errno" -eq 22 ] && [ "$_nt_state" -eq 0 ] && \
		[ "$_tg_errno" -eq 22 ] && [ "$_tg_state" -eq 0 ]; then
		echo "RESULT bind_bad_length=PASS (EINVAL+unbound)"
		PASS=$((PASS + 1))
	else
		echo "RESULT bind_bad_length=FAIL (nt_errno=$_nt_errno nt_state=$_nt_state tg_errno=$_tg_errno tg_state=$_tg_state)"
		FAIL=$((FAIL + 1))
	fi

	_nt_errno=$(bind_field "$_nt" BIND_KEEP_ERRNO)
	_nt_state=$(bind_field "$_nt" BIND_KEEP_STATE)
	_tg_errno=$(bind_field "$_tg" BIND_KEEP_ERRNO)
	_tg_state=$(bind_field "$_tg" BIND_KEEP_STATE)
	[ -n "$_nt_errno" ] || _nt_errno=-1
	[ -n "$_nt_state" ] || _nt_state=-1
	[ -n "$_tg_errno" ] || _tg_errno=-1
	[ -n "$_tg_state" ] || _tg_state=-1
	if [ "$_nt_errno" -eq 0 ] && [ "$_nt_state" -eq 1 ] && \
		[ "$_tg_errno" -eq 0 ] && [ "$_tg_state" -eq 1 ]; then
		echo "RESULT keep_bind_device=PASS (physical bind preserved)"
		PASS=$((PASS + 1))
	else
		echo "RESULT keep_bind_device=FAIL (nt_errno=$_nt_errno nt_state=$_nt_state tg_errno=$_tg_errno tg_state=$_tg_state)"
		FAIL=$((FAIL + 1))
	fi

	# What does an interface that does not exist look like on THIS kernel?
	# EPERM where CAP_NET_RAW is checked before the name is parsed, ENODEV
	# where the name is resolved first.  Hiding an interface means answering
	# exactly this, so the zygisk backend measures the same thing at runtime
	# rather than deriving it from the release string.  Both values are
	# correct; a bind that SUCCEEDS is not, and neither is a bound socket.
	_nt_errno=$(bind_field "$_nt" BIND_ABSENT_ERRNO)
	_nt_state=$(bind_field "$_nt" BIND_ABSENT_STATE)
	[ -n "$_nt_errno" ] || _nt_errno=-1
	[ -n "$_nt_state" ] || _nt_state=-1
	if [ "$_nt_errno" -ne 0 ] && [ "$_nt_state" -eq 0 ]; then
		echo "RESULT bind_absent_name=PASS (errno=$_nt_errno unbound)"
		PASS=$((PASS + 1))
	else
		echo "RESULT bind_absent_name=FAIL (errno=$_nt_errno state=$_nt_state)"
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
check_hide sysfs_stat      "test -e /sys/class/net/vpn0 && echo vpn0" "vpn0"
check_hide sysfs_readdir   "ls /sys/class/net"             "vpn0"
check_hide sysfs_open      "cat /sys/class/net/vpn0/mtu && echo vpn0" "vpn0"
check_hide proc_sys_stat   "test -e /proc/sys/net/ipv4/conf/vpn0 && echo vpn0" "vpn0"
check_hide proc_sys_readdir "ls /proc/sys/net/ipv4/conf"   "vpn0"

_filesystem_hits=$(sed -n '/^0x2710 /s/.* 0x1b:0x\([0-9a-f][0-9a-f]*\).*/\1/p' \
	/proc/vpnhide_ctl | head -1)
case "$_filesystem_hits" in
"" | 0)
	echo "RESULT filesystem_stats=FAIL (hook 0x1b missing or zero)"
	FAIL=$((FAIL + 1))
	;;
*)
	echo "RESULT filesystem_stats=PASS (hook 0x1b count=0x$_filesystem_hits)"
	PASS=$((PASS + 1))
	;;
esac

check_gai
check_ifconf                                                       # sock_ioctl size/tail
check_socket_bind                                                  # interface socket binding

# Non-VPN entries must survive target filtering. This catches over-trimming
# regressions where a hook hides the whole dump instead of only vpn0 rows.
check_keep_exact keep_proc_route_v4   "cat /proc/net/route"     "^eth0"
check_keep_exact keep_getifaddrs      "ip addr show"            ": eth0:"
check_keep_exact keep_siocgifconf     "ifconfig -a"             "^eth0"
check_keep_exact keep_dev_ioctl       "ifconfig eth0"           "^eth0"
check_keep keep_netlink_route4  "ip route show table all" "dev eth0"
check_keep_exact keep_policy_rule     "ip rule show"            "lookup main"
check_keep_exact keep_sysfs_readdir   "ls /sys/class/net"       "eth0"
check_keep_exact keep_proc_sys_readdir "ls /proc/sys/net/ipv4/conf" "eth0"

# Root module managers replace or remove the backend across a reboot. Keep the
# entry-kprobe replacement text resident for that whole lifetime: ordinary
# rmmod must fail and leave both the module and its control plane intact.
# The built-in backend is part of the kernel image — the strongest permanence,
# with no module to rmmod — so the only assertion there is that the control
# plane stays live.
if [ "$VPNHIDE_TEST_BACKEND" = "builtin" ]; then
	if [ -e /proc/vpnhide_ctl ]; then
		echo "RESULT permanent_module=PASS (built-in; no module to remove, control plane live)"
		PASS=$((PASS + 1))
	else
		echo "RESULT permanent_module=FAIL (built-in control plane missing)"
		FAIL=$((FAIL + 1))
	fi
elif rmmod vpnhide_kmod >/tmp/vpnhide-rmmod.log 2>&1; then
	echo "RESULT permanent_module=FAIL (rmmod unexpectedly succeeded)"
	FAIL=$((FAIL + 1))
elif [ -d /sys/module/vpnhide_kmod ] && [ -e /proc/vpnhide_ctl ]; then
	_rmmod_error=$(tr '\n' ' ' </tmp/vpnhide-rmmod.log)
	echo "RESULT permanent_module=PASS (${_rmmod_error:-removal refused})"
	PASS=$((PASS + 1))
else
	_rmmod_error=$(tr '\n' ' ' </tmp/vpnhide-rmmod.log)
	echo "RESULT permanent_module=FAIL (module/control plane missing after refusal; ${_rmmod_error:-no rmmod diagnostic})"
	FAIL=$((FAIL + 1))
fi

PANIC=$(dmesg | grep -ci 'Unable to handle\|Internal error\|Oops\|BUG:\|Kernel panic')
echo "PANIC=$PANIC"
echo "SUMMARY pass=$PASS fail=$FAIL panic=$PANIC registered=$REGISTERED"
echo "##### VPNHIDE-QEMU-TEST END #####"

poweroff -f
