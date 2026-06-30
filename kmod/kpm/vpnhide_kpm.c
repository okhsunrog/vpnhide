// SPDX-License-Identifier: MIT
/*
 * vpnhide — KernelPatch Module (KPM) backend.  *** WIP ***
 *
 * A third native backend alongside the kretprobe `.ko`, for kernels the
 * `.ko` can't serve: non-GKI / proprietary kernels with no published
 * headers or Module.symvers (e.g. kernel 4.14 — issue #33; HyperOS 5.4).
 * Loaded by the KernelPatch runtime (target: KPatch-Next on KernelSU-Next),
 * NOT insmod — so it also works where module signing blocks a `.ko`.
 *
 * STATUS: builds (`make kpm`) and runs end-to-end under QEMU via the KPM
 * harness (../test/run-kpm.sh). All 10 hooks are A/B-validated with no panic
 * (full native-vector parity with the .ko) on FIVE kernels, each a separate
 * from-source QEMU Image: 4.14, 4.19, 5.4, 5.10 and 6.1 (9/9 vectors apiece).
 * The procfs control plane is still TODO (A/B uses load-args). Every per-kver
 * offset must pass the harness before that version ships — a wrong offset is a
 * contained QEMU panic / A/B failure here, but a bootloop on a real device.
 *
 * DESIGN (how this differs from soranerai's prototype, deliberately):
 *   - ONE source + a runtime kver offset table (kver_offsets.h), not three
 *     per-version copies → one binary across 4.x/5.x/6.x.
 *   - Per-call state via `fargs->local.dataN`, NOT a per-CPU MPIDR stash
 *     (which races when a thread migrates between the before/after callback).
 *   - Filtering algorithms shared with the `.ko` via ../shared/vpnhide_logic.h.
 *   - VPN-name matching from the generated single source of truth
 *     (../generated/iface_lists.h → data/interfaces.toml), incl. the `if<N>`
 *     pattern (issue #86) that the hardcoded community lists miss.
 *   - rt_fill_info (single-route lookup) is intentionally NOT hooked: the
 *     QEMU harness proved its arg→register ABI is unstable. soranerai hooks
 *     it; we don't.
 */
#pragma GCC visibility push(hidden)
#include <compiler.h>
#include <common.h> /* kver + VERSION(major,minor,patch) */
#include <kpmodule.h>
#include <hook.h>
#include <kputils.h> /* current_uid() */
#include <log.h>
#include <symbol.h>
#include <kallsyms.h> /* kallsyms_lookup_name (KP-resolved pointer) */
#pragma GCC visibility pop

#include "../generated/iface_lists.h"
#include "../generated/hook_ids.h"
#include "../shared/vpnhide_logic.h"
#include "kver_offsets.h"

KPM_NAME("vpnhide");
KPM_VERSION("0.0.1-wip");
KPM_LICENSE("GPL v2"); /* GPL to use GPL-only kernel symbols at runtime */
KPM_AUTHOR("okhsunrog");
KPM_DESCRIPTION("Hide VPN interfaces from selected UIDs (KPM backend, WIP)");

#define MODNAME "vpnhide"
#define MAX_TARGET_UIDS 64

/* ------------------------------------------------------------------ */
/*  Resolved state                                                    */
/* ------------------------------------------------------------------ */

static const struct vpnhide_offsets *off; /* selected per running kver */

/* Live config (protocol §4.3). Each target carries a per-hook mask so the app
 * can enable hooks individually; a hook fires only if its bit is set for the
 * calling uid.
 *
 * Double-buffered so a hook reader (every hooked syscall, on any CPU) never sees
 * a half-applied target set. A writer (rare, root-initiated: ctl0 config or the
 * init load-args path) fills the INACTIVE buffer, then publishes it with a
 * single release-store of `active_cfg`; readers acquire-load `active_cfg` once
 * and use only that snapshot. KP has no kernel spinlock/RCU, so this is the
 * lock-free equivalent of the .ko's parse-into-staging-then-publish-under-lock.
 * Config writes are assumed serialized (single control path), so two writers
 * never pick the same inactive buffer. */
struct vpnhide_config {
	struct vpnhide_target targets[MAX_TARGET_UIDS];
	int nr_targets;
	uint32_t active_hook_mask;
};
static struct vpnhide_config cfg_buf[2];
static int active_cfg; /* index into cfg_buf; release-published / acquire-read */
static bool debug_enabled;

/* status (protocol §4.3/§5.1): which hooks actually installed, and the dominant
 * fault code. Filled at init, read back via ctl0 `status`. */
static uint32_t installed_hooks;
static uint32_t last_error;

/* Native interception stats, cumulative since KPM load. The KernelPatch build
 * does not use the target kernel's spinlock headers, so slots are reserved with
 * atomic builtins: used=0 empty, 2 initializing, 1 ready. */
static uint32_t stats_used[MAX_TARGET_UIDS];
static uint32_t stats_uids[MAX_TARGET_UIDS];
static unsigned long long stats_counts[MAX_TARGET_UIDS][VPNHIDE_HOOK_COUNT];
static struct vpnhide_stat_entry
	stats_snapshot[MAX_TARGET_UIDS * VPNHIDE_HOOK_COUNT];

/* kernel functions resolved at init via kallsyms */
static void *(*_proc_create_data)(const char *, uint16_t, void *, void *,
				  void *);
static void (*_remove_proc_entry)(const char *, void *);
static int (*_single_open)(void *, void *, void *);
static int (*_single_release)(void *, void *);
static void *_seq_read, *_seq_lseek;
static void (*_seq_printf)(void *, const char *, ...);
static unsigned long (*_copy_from_user)(void *, const void *, unsigned long);
static unsigned long (*_copy_to_user)(void *, const void *, unsigned long);
static void (*_skb_trim)(void *, unsigned int);

#define vpnhide_dbg(fmt, ...)                                   \
	do {                                                    \
		if (debug_enabled)                              \
			logki(MODNAME ": " fmt, ##__VA_ARGS__); \
	} while (0)

/* ------------------------------------------------------------------ */
/*  Core helpers                                                      */
/* ------------------------------------------------------------------ */

/* Fill the inactive buffer from a parsed target set, derive its active-hook
 * mask, then publish it with one release-store. Single-writer (see the cfg_buf
 * comment): a concurrent reader sees either the whole old or whole new config. */
static void publish_config(const struct vpnhide_target *t, int count)
{
	int inactive = 1 - __atomic_load_n(&active_cfg, __ATOMIC_RELAXED);
	struct vpnhide_config *c = &cfg_buf[inactive];
	uint32_t mask = 0;
	int i;

	if (count < 0)
		count = 0;
	if (count > MAX_TARGET_UIDS)
		count = MAX_TARGET_UIDS;
	for (i = 0; i < count; i++) {
		c->targets[i] = t[i];
		mask |= t[i].hookmask & VPNHIDE_KERNEL_HOOK_MASK;
	}
	c->nr_targets = count;
	c->active_hook_mask = mask;
	__atomic_store_n(&active_cfg, inactive, __ATOMIC_RELEASE);
}

/* True if `hook_id` is enabled for the calling uid (per-hook gate, §4.3). Reads
 * the published snapshot once (acquire) so the mask gate and the per-uid scan
 * always come from the same consistent config. */
static int hook_active(uint32_t hook_id)
{
	int b = __atomic_load_n(&active_cfg, __ATOMIC_ACQUIRE);
	const struct vpnhide_config *c = &cfg_buf[b];
	uid_t uid;
	int i;

	if (!(c->active_hook_mask & (1u << hook_id)))
		return 0;
	uid = current_uid();
	for (i = 0; i < c->nr_targets; i++)
		if (c->targets[i].uid == uid)
			return (c->targets[i].hookmask & (1u << hook_id)) != 0;
	return 0;
}

static int stats_slot_for_uid(uint32_t uid)
{
	int i;

	for (i = 0; i < MAX_TARGET_UIDS; i++) {
		if (__atomic_load_n(&stats_used[i], __ATOMIC_ACQUIRE) == 1 &&
		    stats_uids[i] == uid)
			return i;
	}

	for (i = 0; i < MAX_TARGET_UIDS; i++) {
		if (__atomic_load_n(&stats_used[i], __ATOMIC_ACQUIRE) != 0)
			continue;
		if (__sync_bool_compare_and_swap(&stats_used[i], 0, 2)) {
			stats_uids[i] = uid;
			__sync_synchronize();
			__atomic_store_n(&stats_used[i], 1, __ATOMIC_RELEASE);
			return i;
		}
		if (__atomic_load_n(&stats_used[i], __ATOMIC_ACQUIRE) == 1 &&
		    stats_uids[i] == uid)
			return i;
	}

	return -1;
}

static void record_hook_hit(uint32_t hook_id)
{
	int slot;

	if (hook_id >= VPNHIDE_HOOK_COUNT)
		return;
	slot = stats_slot_for_uid((uint32_t)current_uid());
	if (slot >= 0)
		__sync_fetch_and_add(&stats_counts[slot][hook_id], 1ULL);
}

static int snapshot_stats(struct vpnhide_stat_entry *out, int max)
{
	int i, hook, n = 0;

	for (i = 0; i < MAX_TARGET_UIDS && n < max; i++) {
		if (__atomic_load_n(&stats_used[i], __ATOMIC_ACQUIRE) != 1)
			continue;
		for (hook = 0; hook < VPNHIDE_HOOK_COUNT && n < max; hook++) {
			unsigned long long count = __atomic_load_n(
				&stats_counts[i][hook], __ATOMIC_RELAXED);

			if (count == 0)
				continue;
			out[n].uid = stats_uids[i];
			out[n].hook_id = (unsigned int)hook;
			out[n].count = count;
			n++;
		}
	}
	return n;
}

/* NUL-safe copy of a kernel iface name, then match via the generated rules. */
static int iface_is_vpn(const char *name)
{
	char buf[VPNHIDE_IFNAMSIZ];
	int i;

	if (!name)
		return 0;
	for (i = 0; i < VPNHIDE_IFNAMSIZ - 1 && name[i]; i++)
		buf[i] = name[i];
	buf[i] = '\0';
	return vpnhide_iface_is_vpn(buf);
}

/* Read net_device.name given a net_device* (name is at offset 0 everywhere). */
static const char *netdev_name(void *dev)
{
	return dev ? (const char *)((char *)dev + off->netdev_name) : 0;
}

/* True when `dev` (a route's output device) is physical AND the route is a
 * public /32 host-route — the route a VPN client pins to the uplink so tunnel
 * packets can reach the server, which leaks the server's public IPv4 even when
 * the tun interface itself is hidden. The address/iface logic is shared with
 * the .ko (vpnhide_is_public_ipv4 / vpnhide_iface_is_physical in
 * shared/vpnhide_logic.h); only the kernel-struct read is KPM-specific.
 *
 * Only valid on the 5.6+ fib_rt_info path: `fri` is
 * `fargs->args[fib_dump_fi_arg]`, with dst (__be32) at a constant +12 and
 * dst_len (int) at +16 (struct fib_rt_info is stable across GKI 5.10..6.12,
 * verified against the kernel sources). The fri is stack-resident in the
 * caller, so reading those two fields is in-bounds. */
static int kpm_is_public_host_route4(const void *fri, void *dev)
{
	int dst_len;

	if (!fri || !dev)
		return 0;
	dst_len = *(const int *)((const char *)fri + 16);
	if (dst_len != 32)
		return 0;
	if (!vpnhide_is_public_ipv4((const unsigned char *)fri + 12))
		return 0;
	return vpnhide_iface_is_physical(netdev_name(dev));
}

/* IPv6 analogue of kpm_is_public_host_route4: a public /128 host-route pinned to
 * a physical uplink (the route a VPN client installs to reach the server, which
 * leaks its IPv6 even when the tun is hidden — the .ko's
 * is_public_host_route6_via_physical). Reads fib6_info.fib6_dst (rt6key { addr@0;
 * int plen@16 }) at the per-kver offset; off->fib6_info_fib6_dst == 0 disables
 * it (the pre-fib6_info 4.14 rt6_info path, and non-GKI kernels the QEMU matrix
 * can't validate). `rt` is the fib6_info* arg to rt6_fill_node and fib6_dst sits
 * before fib6_nh — which dev_from_fib6_info already reads — so it is in-bounds.
 * The address/iface logic is shared with the .ko (shared/vpnhide_logic.h). */
static int kpm_is_public_host_route6(void *rt, void *dev)
{
	const unsigned char *dst;
	int plen;

	if (!rt || !dev || !off->fib6_info_fib6_dst)
		return 0;
	dst = (const unsigned char *)rt + off->fib6_info_fib6_dst;
	plen = *(const int *)(dst + 16); /* rt6key.plen */
	if (plen != 128)
		return 0;
	if (!vpnhide_is_public_ipv6(dst)) /* rt6key.addr @ +0 */
		return 0;
	return vpnhide_iface_is_physical(netdev_name(dev));
}

/* ================================================================== */
/*  Hook 1 (PoC): fib_route_seq_show — /proc/net/route                */
/*  arg0 = struct seq_file *.  Compact VPN lines out of this call's    */
/*  newly-written region using the shared seq-line compactor.          */
/* ================================================================== */

static void fib_route_before(hook_fargs2_t *fargs, void *udata)
{
	/* Stash seq_file->count at entry so the after-callback only touches
	 * THIS call's output, not earlier entries. (Correctness fix the .ko
	 * already does; soranerai re-scans the whole buffer each call.) */
	void *seq = (void *)fargs->arg0;
	unsigned long count =
		seq ? *(unsigned long *)((char *)seq + off->seqfile_count) : 0;
	fargs->local.data0 = (uint64_t)count;
}

static void fib_route_after(hook_fargs2_t *fargs, void *udata)
{
	void *seq = (void *)fargs->arg0;
	char *buf;
	unsigned long *countp;
	unsigned long start = (unsigned long)fargs->local.data0;

	if (!seq || !hook_active(VPNHIDE_HOOK_FIB_ROUTE_SEQ_SHOW))
		return;

	buf = *(char **)((char *)seq + off->seqfile_buf);
	countp = (unsigned long *)((char *)seq + off->seqfile_count);
	{
		unsigned long old = *countp;
		unsigned long next = vpnhide_compact_seq_lines(
			buf, start, old, VPNHIDE_FIELD_FIRST, iface_is_vpn);

		*countp = next;
		if (next != old)
			record_hook_hit(VPNHIDE_HOOK_FIB_ROUTE_SEQ_SHOW);
	}
}

/* ipv6_route_seq_show — /proc/net/ipv6_route. Same as fib_route but the iface
 * name is the LAST field. Shares fib_route_before (stashes seq->count). */
static void ipv6_route_after(hook_fargs2_t *fargs, void *udata)
{
	void *seq = (void *)fargs->arg0;
	char *buf;
	unsigned long *countp;
	unsigned long start = (unsigned long)fargs->local.data0;

	if (!seq || !hook_active(VPNHIDE_HOOK_IPV6_ROUTE_SEQ_SHOW))
		return;

	buf = *(char **)((char *)seq + off->seqfile_buf);
	countp = (unsigned long *)((char *)seq + off->seqfile_count);
	{
		unsigned long old = *countp;
		unsigned long next = vpnhide_compact_seq_lines(
			buf, start, old, VPNHIDE_FIELD_LAST, iface_is_vpn);

		*countp = next;
		if (next != old)
			record_hook_hit(VPNHIDE_HOOK_IPV6_ROUTE_SEQ_SHOW);
	}
}

/* ================================================================== */
/*  Hook 2 (PoC): rtnl_fill_ifinfo — RTM_NEWLINK (getifaddrs path)    */
/*  arg0 = skb, arg1 = net_device.  If the dev is a VPN iface and the  */
/*  caller is a target, undo whatever the fill wrote (skb_trim back to  */
/*  the saved length) and return 0 — same approach as the .ko.         */
/*  We do NOT return -EMSGSIZE (infinite retry on 6.1 — issue #38).    */
/* ================================================================== */

static void rtnl_fill_before(hook_fargs12_t *fargs, void *udata)
{
	void *skb = (void *)fargs->arg0;
	void *dev = (void *)fargs->arg1;

	fargs->local.data0 = 0; /* should_filter */
	if (!hook_active(VPNHIDE_HOOK_RTNL_FILL_IFINFO) || !skb || !dev)
		return;
	if (!iface_is_vpn(netdev_name(dev)))
		return;

	fargs->local.data0 = 1; /* filter */
	fargs->local.data1 = (uint64_t)skb;
	fargs->local.data2 =
		(uint64_t) * (unsigned int *)((char *)skb + off->skb_len);
}

static void rtnl_fill_after(hook_fargs12_t *fargs, void *udata)
{
	if (!fargs->local.data0)
		return;
	if ((long)fargs->ret < 0)
		return; /* the fill already failed; nothing to undo */
	if (_skb_trim)
		_skb_trim((void *)fargs->local.data1,
			  (unsigned int)fargs->local.data2);
	fargs->ret = 0;
	record_hook_hit(VPNHIDE_HOOK_RTNL_FILL_IFINFO);
}

/* ================================================================== */
/*  Hook 4: dev_ioctl — per-interface ioctls (SIOCGIF* by name)        */
/*  arg1 = cmd, arg2 = ifr (ifr_name at offset 0, uapi-stable). NOTE:   */
/*  arg2 is a *kernel* struct ifreq* on >=5.5, but a *userspace* ptr on */
/*  <5.5 (4.14/4.19/5.4 do the copy inside dev_ioctl). Dereferencing a  */
/*  user ptr directly from kernel context faults under PAN on real HW   */
/*  (QEMU without PAN didn't), so the name is read through the right     */
/*  path for whichever the pointer is. If it names a VPN iface, -ENODEV. */
/* ================================================================== */

#define VPNHIDE_ENODEV ((uint64_t)(-19))

/* arm64: TTBR1 (kernel) addresses have the top 16 bits set; user ptrs don't. */
static int ptr_is_kernel(const void *p)
{
	return ((unsigned long)p >> 48) == 0xffffUL;
}

/* SIOCGIF* range (0x8910..0x8930). SIOCGIFCONF (0x8912) goes via sock_ioctl,
 * never dev_ioctl, so the overlap is harmless here. */
static int is_siocgif(unsigned long cmd)
{
	return cmd >= 0x8910 && cmd <= 0x8930;
}

static void dev_ioctl_after(hook_fargs5_t *fargs, void *udata)
{
	unsigned long cmd = (unsigned long)fargs->arg1;
	const char *ifr = (const char *)fargs->arg2; /* ifr_name @ offset 0 */
	char name[VPNHIDE_IFNAMSIZ];
	int is_vpn;

	if ((long)fargs->ret != 0 || !ifr)
		return;
	if (!hook_active(VPNHIDE_HOOK_DEV_IOCTL) || !is_siocgif(cmd))
		return;

	if (ptr_is_kernel(ifr)) {
		is_vpn = iface_is_vpn(ifr); /* >=5.5: ifr is kernel memory */
	} else {
		/* <5.5: ifr is a __user pointer — copy the name in safely. */
		if (!_copy_from_user ||
		    _copy_from_user(name, ifr, VPNHIDE_IFNAMSIZ))
			return;
		name[VPNHIDE_IFNAMSIZ - 1] = '\0';
		is_vpn = iface_is_vpn(name);
	}
	if (is_vpn) {
		vpnhide_dbg("dev_ioctl: hiding cmd=0x%lx\n", cmd);
		fargs->ret = VPNHIDE_ENODEV;
		record_hook_hit(VPNHIDE_HOOK_DEV_IOCTL);
	}
}

/* ================================================================== */
/*  Hook 5: sock_ioctl — SIOCGIFCONF enumeration                       */
/*  arg1 = cmd, arg2 = userspace struct ifconf*. After the call, compact */
/*  VPN entries out of the returned ifreq[] array. All uapi-stable:      */
/*  struct ifconf { int ifc_len; <pad>; ptr ifc_req@8 }, sizeof ifreq=40,*/
/*  ifr_name @ offset 0.                                                */
/* ================================================================== */

#define VPNHIDE_SIOCGIFCONF 0x8912
#define VPNHIDE_IFREQ_SZ 40 /* sizeof(struct ifreq) on arm64 */

static int filter_ifconf(void *uifc)
{
	char ifc[16]; /* struct ifconf snapshot: len@0 (int), req@8 (ptr) */
	char e[VPNHIDE_IFREQ_SZ];
	char *req;
	int len, n, i, dst = 0;

	if (!_copy_from_user || !_copy_to_user)
		return 0;
	if (_copy_from_user(ifc, uifc, sizeof(ifc)))
		return 0;
	len = *(int *)ifc;
	req = *(char **)(ifc + 8);
	if (!req || len <= 0)
		return 0;

	n = len / VPNHIDE_IFREQ_SZ;
	for (i = 0; i < n; i++) {
		if (_copy_from_user(e, req + (long)i * VPNHIDE_IFREQ_SZ,
				    VPNHIDE_IFREQ_SZ))
			return 0;
		e[VPNHIDE_IFNAMSIZ - 1] = '\0';
		if (iface_is_vpn(e))
			continue; /* drop VPN entry */
		if (dst != i &&
		    _copy_to_user(req + (long)dst * VPNHIDE_IFREQ_SZ, e,
				  VPNHIDE_IFREQ_SZ))
			return 0;
		dst++;
	}
	if (dst != n) {
		int newlen = dst * VPNHIDE_IFREQ_SZ;

		if (_copy_to_user(uifc, &newlen, sizeof(newlen)))
			return 0; /* failed to shrink ifc_len */
		vpnhide_dbg("sock_ioctl: ifconf %d -> %d\n", len, newlen);
		return 1;
	}
	return 0;
}

static void sock_ioctl_after(hook_fargs3_t *fargs, void *udata)
{
	unsigned long cmd = (unsigned long)fargs->arg1;
	void *argp = (void *)fargs->arg2;

	if ((long)fargs->ret != 0 || !argp)
		return;
	if (cmd != VPNHIDE_SIOCGIFCONF || !hook_active(VPNHIDE_HOOK_SOCK_IOCTL))
		return;
	if (filter_ifconf(argp))
		record_hook_hit(VPNHIDE_HOOK_SOCK_IOCTL);
}

/* ================================================================== */
/*  Hooks 9-10: inet_fill_ifaddr / inet6_fill_ifaddr — RTM_GETADDR     */
/*  arg0 = skb, arg1 = ifa.  getifaddrs() enumerates addresses via      */
/*  RTM_GETADDR even when the link (rtnl_fill_ifinfo) is hidden, so      */
/*  these close the address path. dev = ifa->{ifa_dev|idev}->dev.       */
/* ================================================================== */

/* p = *(*(base+off1)+off2) with NULL guards (two-pointer deref). */
static void *deref2(void *base, unsigned int off1, unsigned int off2)
{
	void *p;

	if (!base)
		return 0;
	p = *(void **)((char *)base + off1);
	if (!p)
		return 0;
	return *(void **)((char *)p + off2);
}

/* Shared by both addr-fill hooks: stash skb + len if ifa's dev is VPN. The
 * caller passes its own hook id so the per-hook gate (§4.3) is per-hook even
 * though the body is shared. */
static void addr_fill_before(hook_fargs4_t *fargs, void *dev, uint32_t hook_id)
{
	void *skb = (void *)fargs->arg0;

	fargs->local.data0 = 0;
	if (!hook_active(hook_id) || !skb || !dev)
		return;
	if (!iface_is_vpn(netdev_name(dev)))
		return;
	fargs->local.data0 = 1;
	fargs->local.data1 = (uint64_t)skb;
	fargs->local.data2 =
		(uint64_t) * (unsigned int *)((char *)skb + off->skb_len);
}

static void addr_fill_after_hook(hook_fargs4_t *fargs, uint32_t hook_id)
{
	if (!fargs->local.data0)
		return;
	if ((long)fargs->ret < 0)
		return;
	if (_skb_trim)
		_skb_trim((void *)fargs->local.data1,
			  (unsigned int)fargs->local.data2);
	fargs->ret = 0;
	record_hook_hit(hook_id);
}

static void inet_fill_after(hook_fargs4_t *fargs, void *udata)
{
	addr_fill_after_hook(fargs, VPNHIDE_HOOK_INET_FILL_IFADDR);
}

static void inet6_fill_after(hook_fargs4_t *fargs, void *udata)
{
	addr_fill_after_hook(fargs, VPNHIDE_HOOK_INET6_FILL_IFADDR);
}

static void inet_fill_before(hook_fargs4_t *fargs, void *udata)
{
	void *dev = deref2((void *)fargs->arg1, off->in_ifaddr_ifa_dev,
			   off->in_device_dev);
	addr_fill_before(fargs, dev, VPNHIDE_HOOK_INET_FILL_IFADDR);
}

static void inet6_fill_before(hook_fargs4_t *fargs, void *udata)
{
	void *dev = deref2((void *)fargs->arg1, off->inet6_ifaddr_idev,
			   off->inet6_dev_dev);
	addr_fill_before(fargs, dev, VPNHIDE_HOOK_INET6_FILL_IFADDR);
}

/* ================================================================== */
/*  Hook 6: fib_dump_info — IPv4 RTM_GETROUTE dump (issue #86)         */
/*  arg0 = skb; the fib_info arg index varies by version (table-driven).*/
/*  Resolve the route's output dev (fib_nh[0].nh_common.nhc_dev for a   */
/*  legacy single-nexthop route) and, if it's a VPN iface, undo the     */
/*  fill (skb_trim) + ret 0.                                            */
/*  This is the first hook that dereferences version-specific kernel    */
/*  structs — offsets live in kver_offsets.h, validated by the harness. */
/* ================================================================== */

/* net_device* for a route's fib_info (legacy single-nexthop path only). */
static void *dev_from_fib_info(void *fi)
{
	void *nh;
	int nhs;

	if (!fi || !off->fib_info_fib_nh)
		return 0;
	/* A non-NULL nexthop object means an `ip nexthop`-style route, whose
	 * dev lives behind a separate struct nexthop walk — not unpacked yet.
	 * fib_info_nh == 0 means this version has no nexthop-object field at all
	 * (e.g. 4.14), so skip the check rather than misread fib_info's head. */
	if (off->fib_info_nh) {
		nh = *(void **)((char *)fi + off->fib_info_nh);
		if (nh)
			return 0;
	}
	nhs = *(int *)((char *)fi + off->fib_info_fib_nhs);
	if (nhs <= 0)
		return 0;
	/* nhc_dev is the first member of fib_nh[0] (== fib_nh_common). */
	return *(void **)((char *)fi + off->fib_info_fib_nh);
}

/*
 * fib_dump_info's prototype moved the fib_info across versions, so the arg
 * index + how to reach the fib_info are table-driven (kver_offsets.h):
 *   - 5.6+ : fib_dump_info(skb, portid, seq, event, struct fib_rt_info *fri,
 *            flags) — fi = fri->fi, fi_arg=4, via_fri=1.
 *   - <5.6 : fib_dump_info(skb, portid, seq, event, tb_id, type, dst, dst_len,
 *            tos, struct fib_info *fi, flags) — fi_arg=9, via_fri=0.
 * Always hooked as a 12-arg frame (argno=11): KP just saves the extra slots,
 * the same way rtnl_fill_ifinfo is over-specified — only the fi arg is read.
 */
static void fib_dump_before(hook_fargs12_t *fargs, void *udata)
{
	void *skb = (void *)fargs->arg0;
	void *p = (void *)fargs->args[off->fib_dump_fi_arg];
	void *fi, *dev;

	fargs->local.data0 = 0;
	if (!hook_active(VPNHIDE_HOOK_FIB_DUMP_INFO) || !skb || !p)
		return;
	fi = off->fib_dump_fi_via_fri ? *(void **)p : p; /* fib_rt_info.fi @0 */
	dev = dev_from_fib_info(fi);
	if (!dev)
		return;
	/* Hide the route if its output dev is a VPN iface, OR (5.6+ only, where
	 * dst/dst_len live at fixed offsets in the fib_rt_info) it is a public
	 * /32 host-route pinned to a physical uplink — the .ko hides both, so the
	 * KPM must too for backend parity. */
	if (!iface_is_vpn(netdev_name(dev)) &&
	    !(off->fib_dump_fi_via_fri && kpm_is_public_host_route4(p, dev)))
		return;

	fargs->local.data0 = 1;
	fargs->local.data1 = (uint64_t)skb;
	fargs->local.data2 =
		(uint64_t) * (unsigned int *)((char *)skb + off->skb_len);
}

static void fib_dump_after(hook_fargs12_t *fargs, void *udata)
{
	if (!fargs->local.data0)
		return;
	if ((long)fargs->ret < 0)
		return;
	if (_skb_trim)
		_skb_trim((void *)fargs->local.data1,
			  (unsigned int)fargs->local.data2);
	fargs->ret = 0;
	record_hook_hit(VPNHIDE_HOOK_FIB_DUMP_INFO);
}

/* ================================================================== */
/*  Hook 7: rt6_fill_node — IPv6 RTM_GETROUTE dump                     */
/*  arg1 = skb, arg2 = fib6_info*.  IPv6 analogue of fib_dump_info.     */
/* ================================================================== */

static void *dev_from_fib6_info(void *rt)
{
	void *nh;

	if (!rt)
		return 0;
	/* Pre-fib6_info kernels: rt is a struct rt6_info* whose embedded
	 * dst_entry holds the dev directly (no nexthop walk). */
	if (off->rt6_via_dst)
		return *(void **)((char *)rt + off->rt6_dst_dev);
	if (!off->fib6_info_fib6_nh)
		return 0;
	/* fib6_info_nh == 0 => this version has no nexthop-object field (e.g.
	 * 4.19/4.14); skip the check rather than misread fib6_info's head. */
	if (off->fib6_info_nh) {
		nh = *(void **)((char *)rt + off->fib6_info_nh);
		if (nh)
			return 0; /* nexthop-object route — not unpacked yet */
	}
	return *(void **)((char *)rt + off->fib6_info_fib6_nh);
}

static void rt6_fill_before(hook_fargs12_t *fargs, void *udata)
{
	void *skb = (void *)fargs->arg1;
	void *rt = (void *)fargs->arg2;
	void *dev;

	fargs->local.data0 = 0;
	if (!hook_active(VPNHIDE_HOOK_RT6_FILL_NODE) || !skb || !rt)
		return;
	dev = dev_from_fib6_info(rt);
	if (!dev)
		return;
	/* Hide the route if its output dev is a VPN iface, OR it is a public /128
	 * host-route pinned to a physical uplink (parity with the .ko). */
	if (!iface_is_vpn(netdev_name(dev)) &&
	    !kpm_is_public_host_route6(rt, dev))
		return;

	fargs->local.data0 = 1;
	fargs->local.data1 = (uint64_t)skb;
	fargs->local.data2 =
		(uint64_t) * (unsigned int *)((char *)skb + off->skb_len);
}

static void rt6_fill_after(hook_fargs12_t *fargs, void *udata)
{
	if (!fargs->local.data0)
		return;
	if ((long)fargs->ret < 0)
		return;
	if (_skb_trim)
		_skb_trim((void *)fargs->local.data1,
			  (unsigned int)fargs->local.data2);
	fargs->ret = 0;
	record_hook_hit(VPNHIDE_HOOK_RT6_FILL_NODE);
}

/* ================================================================== */
/*  Hook 8: fib_nl_fill_rule — RTM_GETRULE (policy routing rules)      */
/*  arg0 = skb, arg1 = fib_rule*.  Hide a rule if it routes via a VPN   */
/*  iface (iif/oifname) or selects a non-standard table for a target    */
/*  UID range — the per-UID VPN policy rules.                          */
/* ================================================================== */

static void fib_rule_before(hook_fargs8_t *fargs, void *udata)
{
	void *skb = (void *)fargs->arg0;
	char *rule = (char *)fargs->arg1;
	const char *iif, *oif;
	int filter = 0;

	fargs->local.data0 = 0;
	if (!hook_active(VPNHIDE_HOOK_FIB_NL_FILL_RULE) || !skb || !rule ||
	    !off->fib_rule_table)
		return;

	iif = rule + off->fib_rule_iifname;
	oif = rule + off->fib_rule_oifname;
	if ((iif[0] && iface_is_vpn(iif)) || (oif[0] && iface_is_vpn(oif))) {
		filter = 1;
	} else {
		uint32_t table = *(uint32_t *)(rule + off->fib_rule_table);
		uint32_t start = *(uint32_t *)(rule + off->fib_rule_uid_start);
		uint32_t end = *(uint32_t *)(rule + off->fib_rule_uid_end);
		uint32_t uid = (uint32_t)current_uid();

		if (uid >= start && uid <= end &&
		    (start != 0 || end != 0xffffffffu) && table != 253 &&
		    table != 254 && table != 255 && table > 100)
			filter = 1;
	}

	if (filter) {
		fargs->local.data0 = 1;
		fargs->local.data1 = (uint64_t)skb;
		fargs->local.data2 =
			(uint64_t) *
			(unsigned int *)((char *)skb + off->skb_len);
	}
}

static void fib_rule_after(hook_fargs8_t *fargs, void *udata)
{
	if (!fargs->local.data0)
		return;
	if ((long)fargs->ret < 0)
		return;
	if (_skb_trim)
		_skb_trim((void *)fargs->local.data1,
			  (unsigned int)fargs->local.data2);
	fargs->ret = 0;
	record_hook_hit(VPNHIDE_HOOK_FIB_NL_FILL_RULE);
}

/*
 * HOOK COVERAGE — full parity with vpnhide_kmod.c (the .ko). All 10 hooks
 * ported and QEMU-validated A/B on android12-5.10 (no panic). Mirror the
 * .ko's logic; reuse shared/vpnhide_logic.h. Per-version struct offsets live
 * in kver_offsets.h (5.10 only so far) — a wrong offset is a contained QEMU
 * A/B fail / panic here, a bootloop on a real device.
 *
 *   fib_route_seq_show     /proc/net/route        ✓ (seq compactor)
 *   ipv6_route_seq_show    /proc/net/ipv6_route   ✓ (seq compactor)
 *   rtnl_fill_ifinfo       RTM_GETLINK            ✓ (skb.len)
 *   inet_fill_ifaddr       RTM_GETADDR v4         ✓ (in_ifaddr.ifa_dev->dev)
 *   inet6_fill_ifaddr      RTM_GETADDR v6         ✓ (inet6_ifaddr.idev->dev)
 *   dev_ioctl              SIOCGIF* by name       ✓ (ret -> -ENODEV)
 *   sock_ioctl             SIOCGIFCONF            ✓ (ifconf compaction)
 *   fib_dump_info          RTM_GETROUTE v4 dump   ✓ (#86; fib_info nexthop +
 *                                                   public /32 host-route via a
 *                                                   physical uplink, the .ko's
 *                                                   is_public_host_route_via_
 *                                                   physical — constant
 *                                                   fib_rt_info offsets, A/B on
 *                                                   5.10 + 6.12)
 *   rt6_fill_node          RTM_GETROUTE v6 dump   ✓ (fib6_info nexthop +
 *                                                   public /128 host-route via a
 *                                                   physical uplink, the .ko's
 *                                                   is_public_host_route6_via_
 *                                                   physical — per-kver
 *                                                   fib6_info.fib6_dst offset,
 *                                                   A/B on 5.10/5.15/6.1/6.12)
 *   fib_nl_fill_rule       RTM_GETRULE            ✓ (fib_rule iif/oif/uid)
 *   ( rt_fill_info — intentionally NOT hooked; unstable arg->reg ABI )
 *
 * Both host-route predicates (v4 + v6) and their address/iface logic are now
 * shared with the .ko via shared/vpnhide_logic.h. The IPv4 path needs no offset
 * (fib_rt_info.dst/dst_len are at a constant +12/+16 on 5.6+); the IPv6 path
 * reads fib6_info.fib6_dst at the per-kver offset (64 on 5.10..6.6, 80 on 6.12),
 * disabled (0) on the non-GKI kernels the QEMU matrix can't validate.
 */

/* ------------------------------------------------------------------ */
/*  Control plane                                                      */
/* ------------------------------------------------------------------ */

/*
 * The KPM has no file or node: its control/stats channel is the KernelPatch
 * `ctl0` supercall, carrying the §4 wire format (config in, stats/status out —
 * see vpnhide_kpm_ctl0). A procfs mirror is intentionally NOT created: it would
 * need the version-specific proc_ops/file_operations ABI guessed without real
 * headers (the likely cause of the HyperOS-5.4 crash report), and ctl0 needs no
 * such guessing.
 *
 * proc create/remove glue stub left below for the (deferred) optional mirror.
 */

/*
 * proc_create needs a `struct proc_ops` (>=5.6) or a `struct file_operations`
 * (<5.6). We don't have the real headers, so the field layout is a mock —
 * and getting it wrong is the likely cause of the HyperOS-5.4 crash report.
 * off->proc_uses_proc_ops selects which mock to register.
 * TODO: define both mock layouts + the targets/debug show/open handlers, and
 * register the matching one. Kept as a stub here so the skeleton stays focused
 * on the hooks; see kver_offsets.h for the ABI flag.
 */

/* ------------------------------------------------------------------ */
/*  Init / exit                                                       */
/* ------------------------------------------------------------------ */

/*
 * Resolve a hook target by name, tolerating compiler-renamed clones. GCC may
 * emit a static function as `name.isra.N` / `name.constprop.N` (a specialised
 * clone) — kallsyms_lookup_name("name") then misses it. Android *device*
 * kernels are clang-built (name intact), but a gcc-built kernel (incl. our
 * QEMU test kernels) renames e.g. fib_nl_fill_rule -> fib_nl_fill_rule.isra.N.
 * Fall back to the first symbol equal to `name`, or `name.` + suffix — the
 * dot only appears on compiler clones, so this never matches an unrelated fn.
 */
struct vpnhide_sym_q {
	const char *base;
	int baselen;
	unsigned long addr;
};

static int vpnhide_sym_cb(void *data, const char *name, struct module *mod,
			  unsigned long addr)
{
	struct vpnhide_sym_q *q = data;
	int i;

	(void)mod;
	for (i = 0; i < q->baselen; i++)
		if (name[i] != q->base[i])
			return 0;
	if (name[q->baselen] == '\0' || name[q->baselen] == '.') {
		q->addr = addr;
		return 1; /* found — stop iterating */
	}
	return 0;
}

static unsigned long lookup_fn(const char *name)
{
	unsigned long fn = kallsyms_lookup_name(name);
	struct vpnhide_sym_q q;

	if (fn)
		return fn;
	if (!kallsyms_on_each_symbol)
		return 0;
	q.base = name;
	q.baselen = 0;
	while (name[q.baselen])
		q.baselen++;
	q.addr = 0;
	kallsyms_on_each_symbol(vpnhide_sym_cb, &q);
	return q.addr;
}

static int resolve_symbols(void)
{
	_proc_create_data = (void *)kallsyms_lookup_name("proc_create_data");
	_remove_proc_entry = (void *)kallsyms_lookup_name("remove_proc_entry");
	_single_open = (void *)kallsyms_lookup_name("single_open");
	_single_release = (void *)kallsyms_lookup_name("single_release");
	_seq_read = (void *)kallsyms_lookup_name("seq_read");
	_seq_lseek = (void *)kallsyms_lookup_name("seq_lseek");
	_seq_printf = (void *)kallsyms_lookup_name("seq_printf");

	/* Prefer the generic `_copy_*_user` wrappers, NOT the raw
	 * `__arch_copy_*_user`. The wrapper does the uaccess enable/disable
	 * (access_ok + the TTBR0 switch / PAN toggle) around the copy; the raw
	 * asm only copies. On a kernel using software PAN
	 * (CONFIG_ARM64_SW_TTBR0_PAN — old ARMv8.0 cores with no hardware PAN,
	 * and the QEMU harness on `-cpu cortex-a57`) the TTBR0 switch lives in
	 * the C wrapper on >=5.x, so calling the raw routine directly faults on
	 * the unmapped user page (caught by the 5.4 harness run). The wrapper is
	 * correct under both hardware and software PAN; fall back to the raw
	 * symbol only if the wrapper is absent. */
	_copy_from_user = (void *)kallsyms_lookup_name("_copy_from_user");
	if (!_copy_from_user)
		_copy_from_user =
			(void *)kallsyms_lookup_name("__arch_copy_from_user");

	_copy_to_user = (void *)kallsyms_lookup_name("_copy_to_user");
	if (!_copy_to_user)
		_copy_to_user =
			(void *)kallsyms_lookup_name("__arch_copy_to_user");

	_skb_trim = (void *)kallsyms_lookup_name("__skb_trim");
	if (!_skb_trim)
		_skb_trim = (void *)kallsyms_lookup_name("skb_trim");

	/* Hooks need these; proc is best-effort. */
	return _skb_trim ? 0 : -1;
}

/*
 * Load-time / test target path: a bare newline/space-separated decimal UID list
 * (KernelPatch load extra-args, e.g. sc_kpm_load(key, path, "10010 10020"), as
 * the QEMU A/B harness uses). Each listed uid gets the FULL kernel hook mask —
 * i.e. "enable everything for these uids". Per-hook control is the job of the
 * runtime ctl0 `config` channel (vpnhide_parse_config); this path predates it
 * and stays for headless bring-up where no superkey/ctl0 round-trip is wired.
 */
static void apply_targets(const char *s)
{
	unsigned int uids[MAX_TARGET_UIDS];
	struct vpnhide_target newt[MAX_TARGET_UIDS];
	unsigned long n = 0;
	int cnt, i;

	if (!s)
		return;
	while (s[n])
		n++;
	cnt = vpnhide_parse_target_uids(s, n, uids, MAX_TARGET_UIDS);
	for (i = 0; i < cnt; i++) {
		newt[i].uid = uids[i];
		newt[i].hookmask = VPNHIDE_KERNEL_HOOK_MASK;
	}
	publish_config(newt, cnt);
	vpnhide_dbg("loaded %d target UIDs\n", cnt);
}

/* Resolve `name`, wrap it, and record the install in `installed_hooks` so the
 * status channel (§4.3 `hooks`) reflects what actually took. */
static void install_hook(const char *name, int argno, void *before, void *after,
			 uint32_t hook_id)
{
	unsigned long fn = lookup_fn(name);

	if (!fn)
		return;
	if (hook_wrap((void *)fn, argno, before, after, 0) == HOOK_NO_ERR)
		installed_hooks |= (1u << hook_id);
}

static long vpnhide_kpm_init(const char *args, const char *event,
			     void *__user reserved)
{
	logki(MODNAME ": KPM init (event=%s) kver=0x%x\n", event ? event : "",
	      (unsigned int)kver);

	installed_hooks = 0;
	last_error = VPNHIDE_ERR_OK;
	publish_config(0, 0); /* start with an empty (no-op) active config */

	/* `kver` is KernelPatch's running-kernel version (common.h), encoded
	 * the same way as VPNHIDE_KVER. NULL table = unsupported → bail. */
	off = vpnhide_select_offsets((unsigned int)kver);
	if (!off) {
		logki(MODNAME
		      ": unsupported kernel version — refusing to install\n");
		return -1; /* never guess offsets */
	}
	if (resolve_symbols() != 0) {
		logki(MODNAME ": symbol resolution failed\n");
		return -1;
	}

	/* Targets can come at load time: sc_kpm_load(key, path, "10010 10020")
	 * (decimal list, all hooks). The runtime ctl0 `config` channel feeds the
	 * same set with per-hook masks. */
	apply_targets(args);

	/*
	 * Install hooks. Each one is gated on the offset(s) it dereferences
	 * being known for this kernel version (0 => not installed), so a
	 * partially-filled offset table is SAFE: a hook never runs with a
	 * wrong/zero offset and panics. seq_file + ioctl hooks need only
	 * stable offsets (seqfile_count, uapi ifreq) so they install whenever
	 * the symbol exists. install_hook records each into installed_hooks.
	 */
	if (off->seqfile_count) {
		install_hook("fib_route_seq_show", 2, (void *)fib_route_before,
			     (void *)fib_route_after,
			     VPNHIDE_HOOK_FIB_ROUTE_SEQ_SHOW);
		install_hook("ipv6_route_seq_show", 2, (void *)fib_route_before,
			     (void *)ipv6_route_after,
			     VPNHIDE_HOOK_IPV6_ROUTE_SEQ_SHOW);
	}
	install_hook("dev_ioctl", 5, 0, (void *)dev_ioctl_after,
		     VPNHIDE_HOOK_DEV_IOCTL);
	install_hook("sock_ioctl", 3, 0, (void *)sock_ioctl_after,
		     VPNHIDE_HOOK_SOCK_IOCTL);
	if (off->skb_len)
		install_hook("rtnl_fill_ifinfo", 12, (void *)rtnl_fill_before,
			     (void *)rtnl_fill_after,
			     VPNHIDE_HOOK_RTNL_FILL_IFINFO);
	if (off->in_ifaddr_ifa_dev)
		install_hook("inet_fill_ifaddr", off->addr_fill_argno,
			     (void *)inet_fill_before, (void *)inet_fill_after,
			     VPNHIDE_HOOK_INET_FILL_IFADDR);
	if (off->inet6_ifaddr_idev)
		install_hook("inet6_fill_ifaddr", off->addr_fill_argno,
			     (void *)inet6_fill_before,
			     (void *)inet6_fill_after,
			     VPNHIDE_HOOK_INET6_FILL_IFADDR);
	if (off->fib_dump_fi_arg)
		install_hook("fib_dump_info", 11, (void *)fib_dump_before,
			     (void *)fib_dump_after,
			     VPNHIDE_HOOK_FIB_DUMP_INFO);
	if (off->fib6_info_fib6_nh || off->rt6_via_dst)
		install_hook("rt6_fill_node", 11, (void *)rt6_fill_before,
			     (void *)rt6_fill_after,
			     VPNHIDE_HOOK_RT6_FILL_NODE);
	if (off->fib_rule_table)
		install_hook("fib_nl_fill_rule", 7, (void *)fib_rule_before,
			     (void *)fib_rule_after,
			     VPNHIDE_HOOK_FIB_NL_FILL_RULE);

	/* Healthy iff every kernel-owned hook installed; otherwise honestly
	 * report partial — the `hooks` mask carries which ones (§5.1). A kver
	 * with an incomplete offset table lands here by design. */
	last_error = (installed_hooks == VPNHIDE_KERNEL_HOOK_MASK) ?
			     VPNHIDE_ERR_OK :
			     VPNHIDE_ERR_PARTIAL_HOOKS;

	logki(MODNAME ": KPM hooks installed (mask=0x%x err=%u)\n",
	      installed_hooks, last_error);
	return 0;
}

/* Single fixed reply buffer for stats/status (§7.2 — no pagination). A few KiB
 * holds stats for tens of uids; the reader passes a generous outlen and we
 * truncate on a line boundary (clamp_to_line) if it ever overflows. */
#define VPNHIDE_OUT_MAX 4096

/*
 * Runtime control/stats channel (protocol §7.1). KernelPatch forwards `args`
 * in and `out_msg` (copy_to_user) out; the `long` return is a short code only,
 * never surfaced as text. Dispatch on the header `kind`:
 *   config → apply the snapshot (per-hook masks + debug), return 0.
 *   stats  → serialise cumulative per-uid/per-hook counters into out_msg.
 *   status → serialise backend health into out_msg.
 */
static long vpnhide_kpm_ctl0(const char *args, char *__user out_msg, int outlen)
{
	unsigned long n_args = 0;
	enum vpnhide_kind kind;

	if (!args)
		return -1;
	while (args[n_args])
		n_args++;
	kind = vpnhide_peek_kind(args, n_args);

	if (kind == VPNHIDE_KIND_CONFIG) {
		/* Parse into a stack-local staging set, then publish atomically —
		 * never mutate the live config in place (a hook reader on another
		 * CPU must not see a half-written targets[]). */
		struct vpnhide_target newt[MAX_TARGET_UIDS];
		int dbg = debug_enabled ? 1 : 0;
		int n = vpnhide_parse_config(args, n_args, newt, MAX_TARGET_UIDS,
					     &dbg);

		if (n < 0)
			return -1; /* rejected whole (bad header / version) */
		publish_config(newt, n);
		debug_enabled = dbg ? true : false;
		vpnhide_dbg("ctl0 config: %d targets, debug=%d\n", n, dbg);
		return 0;
	}

	if (kind == VPNHIDE_KIND_STATS || kind == VPNHIDE_KIND_STATUS) {
		char buf[VPNHIDE_OUT_MAX];
		unsigned long full, n;

		if (kind == VPNHIDE_KIND_STATS) {
			int count = snapshot_stats(stats_snapshot,
						   MAX_TARGET_UIDS *
							   VPNHIDE_HOOK_COUNT);

			full = vpnhide_format_stats(buf, sizeof(buf),
						    stats_snapshot, count);
		} else {
			struct vpnhide_status st;

			st.backend = VPNHIDE_BACKEND_KPM;
			st.kver = (unsigned int)kver;
			st.hooks = installed_hooks;
			st.error = last_error;
			full = vpnhide_format_status(buf, sizeof(buf), &st);
		}

		/*
		 * vpnhide_format_* report the FULL intended length, which can
		 * exceed sizeof(buf); the bytes past the buffer were never
		 * written. Clamp before clamp_to_line/copy_to_user so a
		 * generous outlen can't drive a read past the 4096-byte stack
		 * buffer (kernel infoleak / OOB). The .ko bounds the same way.
		 */
		if (full > sizeof(buf))
			full = sizeof(buf);
		n = vpnhide_clamp_to_line(
			buf, full, outlen > 0 ? (unsigned long)outlen : 0);
		if (_copy_to_user && out_msg && n)
			_copy_to_user(out_msg, buf, n);
		return (long)n;
	}

	return -1; /* unknown kind */
}

static long vpnhide_kpm_exit(void *__user reserved)
{
	unsigned long fn;

	fn = lookup_fn("fib_route_seq_show");
	if (fn)
		hook_unwrap((void *)fn, (void *)fib_route_before,
			    (void *)fib_route_after);
	fn = lookup_fn("rtnl_fill_ifinfo");
	if (fn)
		hook_unwrap((void *)fn, (void *)rtnl_fill_before,
			    (void *)rtnl_fill_after);
	fn = lookup_fn("ipv6_route_seq_show");
	if (fn)
		hook_unwrap((void *)fn, (void *)fib_route_before,
			    (void *)ipv6_route_after);
	fn = lookup_fn("inet_fill_ifaddr");
	if (fn)
		hook_unwrap((void *)fn, (void *)inet_fill_before,
			    (void *)inet_fill_after);
	fn = lookup_fn("inet6_fill_ifaddr");
	if (fn)
		hook_unwrap((void *)fn, (void *)inet6_fill_before,
			    (void *)inet6_fill_after);
	fn = lookup_fn("dev_ioctl");
	if (fn)
		hook_unwrap((void *)fn, 0, (void *)dev_ioctl_after);
	fn = lookup_fn("sock_ioctl");
	if (fn)
		hook_unwrap((void *)fn, 0, (void *)sock_ioctl_after);
	fn = lookup_fn("fib_dump_info");
	if (fn)
		hook_unwrap((void *)fn, (void *)fib_dump_before,
			    (void *)fib_dump_after);
	fn = lookup_fn("rt6_fill_node");
	if (fn)
		hook_unwrap((void *)fn, (void *)rt6_fill_before,
			    (void *)rt6_fill_after);
	fn = lookup_fn("fib_nl_fill_rule");
	if (fn)
		hook_unwrap((void *)fn, (void *)fib_rule_before,
			    (void *)fib_rule_after);

	/* No /proc node to remove: the KPM's control channel is the ctl0
	 * supercall, not procfs (the optional mirror was never created). */
	logki(MODNAME ": KPM unloaded\n");
	return 0;
}

KPM_INIT(vpnhide_kpm_init);
KPM_CTL0(vpnhide_kpm_ctl0);
KPM_EXIT(vpnhide_kpm_exit);
