// SPDX-License-Identifier: MIT
/*
 * vpnhide (in-tree) — SO_BINDTODEVICE / SO_BINDTOIFINDEX concealment.
 *
 * Faithful port of the .ko's prepare_socket_bind()/classify_bind_ifindex(), with
 * all of the kretprobe/kprobe machinery removed. The per-version patch calls
 * vpnhide_setsockopt_bind() from __sys_setsockopt() BEFORE the option is applied
 * (process context), so:
 *   - there is no PC-redirect / call-original trampoline (the .ko needed one to
 *     run the mutation in process context after an atomic kprobe pre-handler);
 *   - dev_get_by_index_rcu() is a direct in-scope call (the .ko had to resolve it
 *     through a throwaway kprobe to survive OEMs that trim it from the KMI);
 *   - `level` is the genuine syscall argument at __sys_setsockopt, not the
 *     LTO-elided sk_setsockopt arg, so a SOL_SOCKET gate at the patch site is
 *     safe (the .ko had to drop that gate to avoid the LTO SO_BINDTODEVICE leak).
 *
 * FROZEN still snapshots the option into kernel memory so a racing userspace
 * cannot swap a physical iface name for a VPN one between the check and the
 * handler's own copy_from_sockptr — the same anti-TOCTOU guarantee as the .ko.
 */

#include <linux/kernel.h>
#include <linux/string.h>
#include <linux/if.h>
#include <linux/socket.h>
#include <linux/netdevice.h>
#include <linux/uaccess.h>	/* copy_from_user (pre-5.9 bind variant) */
#include <net/sock.h>

#include <linux/vpnhide.h>

#include "vpnhide_internal.h"
#include "generated/iface_lists.h"
#include "shared/vpnhide_logic.h"

#define is_vpn_ifname(name) vpnhide_iface_is_vpn(name)

/* 1 = VPN interface, 0 = physical/non-VPN, -1 = unknown. Unknown positive
 * indexes fail closed: sock_bindtoindex accepts a non-existent positive index,
 * which would otherwise leave observable state on the socket. ifindex 0 clears
 * the binding and is always allowed. */
static int classify_bind_ifindex(struct sock *sk, int ifindex)
{
	struct net_device *dev;
	int result = -1;

	if (!sk || ifindex <= 0)
		return ifindex == 0 ? 0 : -1;

	rcu_read_lock();
	dev = dev_get_by_index_rcu(sock_net(sk), ifindex);
	if (dev)
		result = is_vpn_ifname(dev->name) ? 1 : 0;
	rcu_read_unlock();
	return result;
}

/* True if this setsockopt is a bind-to-interface option we must classify. The
 * SO_BINDTOIFINDEX comparison is guarded because it only exists from 5.1 (absent
 * on android10-4.19 / android10-4.14). */
static bool bind_opt_relevant(int optname)
{
	if (!vpnhide_hook_active(VPNHIDE_HOOK_SOCKET_BIND_INTERFACE))
		return false;
	if (optname == SO_BINDTODEVICE)
		return true;
#ifdef SO_BINDTOIFINDEX
	if (optname == SO_BINDTOIFINDEX)
		return true;
#endif
	return false;
}

/*
 * Conceal a bind to a VPN interface WITHOUT choosing an errno ourselves. A fixed
 * ENODEV is the native "absent interface" answer only on kernels that resolve
 * the name before the CAP_NET_RAW check; on kernels that check the capability
 * first, an absent interface yields EPERM to an unprivileged caller, so a
 * hardcoded ENODEV for the hidden name would differ from every other name — a
 * fingerprint of the filter. Instead, freeze in a value that can never name a
 * real interface and let the kernel run its normal path: the caller then gets
 * EXACTLY what a genuinely-absent interface returns for their privilege on this
 * kernel (ENODEV, or EPERM where the cap gate fires first). Order-agnostic and
 * indistinguishable, with no per-version errno table.
 *
 * "/" is rejected by dev_valid_name(), so no interface can ever bear it, and it
 * resolves to "no such device" on every kernel.
 *
 * This trick only works for the NAME option: SO_BINDTOIFINDEX stores any
 * non-negative index without a lookup (sock_bindtoindex_locked never returns
 * ENODEV), so there is no "absent index" answer to imitate. The index of a VPN
 * interface is also unlearnable — SIOCGIFINDEX / if_nametoindex / getifaddrs are
 * already concealed — so a caller cannot target it by index in the first place;
 * we simply deny that bind (ENODEV, as the .ko does).
 */
static enum vpnhide_bind_action classify_bind_name(union vpnhide_bind_snapshot *snap)
{
	if (snap->name[0] && is_vpn_ifname(snap->name)) {
		vpnhide_record_hook_hit(VPNHIDE_HOOK_SOCKET_BIND_INTERFACE);
		snap->name[0] = '/';
		snap->name[1] = '\0';
	}
	return VPNHIDE_BIND_FROZEN;
}

static enum vpnhide_bind_action classify_bind_idx(struct sock *sk,
						  union vpnhide_bind_snapshot *snap)
{
	if (snap->ifindex > 0 && classify_bind_ifindex(sk, snap->ifindex) != 0) {
		vpnhide_record_hook_hit(VPNHIDE_HOOK_SOCKET_BIND_INTERFACE);
		return VPNHIDE_BIND_DENY;
	}
	return VPNHIDE_BIND_FROZEN;
}

#ifdef VPNHIDE_HAVE_SOCKPTR
enum vpnhide_bind_action vpnhide_setsockopt_bind(struct sock *sk, int optname,
						 sockptr_t optval,
						 unsigned int optlen,
						 union vpnhide_bind_snapshot *snap)
{
	if (!bind_opt_relevant(optname))
		return VPNHIDE_BIND_PASSTHROUGH;

	memset(snap, 0, sizeof(*snap));

	if (optname == SO_BINDTODEVICE) {
		size_t n;

		/* sock_setbindtodevice takes an int and rejects a negative
		 * optlen before touching optval — preserve EINVAL, avoid uaccess. */
		if ((int)optlen < 0)
			return VPNHIDE_BIND_PASSTHROUGH;
		n = min_t(size_t, optlen, IFNAMSIZ - 1);

		if (n && copy_from_sockptr(snap->name, optval, n))
			return VPNHIDE_BIND_FAULT;
		return classify_bind_name(snap);
	}

	if (optlen < sizeof(snap->ifindex))
		return VPNHIDE_BIND_PASSTHROUGH;
	if (copy_from_sockptr(&snap->ifindex, optval, sizeof(snap->ifindex)))
		return VPNHIDE_BIND_FAULT;
	return classify_bind_idx(sk, snap);
}
#endif /* VPNHIDE_HAVE_SOCKPTR */

/*
 * Pre-5.9 (bare user pointer) variant. Identical decision to the sockptr form,
 * copying from user memory directly. On FROZEN the pre-5.9 __sys_setsockopt
 * patch swaps optval to point at *snap under set_fs(KERNEL_DS) — the same
 * anti-TOCTOU freeze, using the kernel's existing kernel_optval mechanism. The
 * hide itself is the frozen guaranteed-absent value from classify_bind_*, so the
 * kernel's own path produces the native absent errno for the caller's privilege
 * (this is what makes it correct on both cap-first and name-first legacy).
 */
enum vpnhide_bind_action vpnhide_setsockopt_bind_user(struct sock *sk, int optname,
						      const char __user *optval,
						      unsigned int optlen,
						      union vpnhide_bind_snapshot *snap)
{
	if (!bind_opt_relevant(optname))
		return VPNHIDE_BIND_PASSTHROUGH;

	memset(snap, 0, sizeof(*snap));

	if (optname == SO_BINDTODEVICE) {
		size_t n;

		if ((int)optlen < 0)
			return VPNHIDE_BIND_PASSTHROUGH;
		n = min_t(size_t, optlen, IFNAMSIZ - 1);

		if (n && copy_from_user(snap->name, optval, n))
			return VPNHIDE_BIND_FAULT;
		return classify_bind_name(snap);
	}

	if (optlen < sizeof(snap->ifindex))
		return VPNHIDE_BIND_PASSTHROUGH;
	if (copy_from_user(&snap->ifindex, optval, sizeof(snap->ifindex)))
		return VPNHIDE_BIND_FAULT;
	return classify_bind_idx(sk, snap);
}
