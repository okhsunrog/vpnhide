// SPDX-License-Identifier: MIT
/*
 * vpnhide — KernelPatch Module (KPM) backend.  *** BETA ***
 *
 * A third native backend alongside the kretprobe `.ko`, for kernels the
 * `.ko` can't serve: non-GKI / proprietary kernels with no published
 * headers or Module.symvers (e.g. kernel 4.14 — issue #33; HyperOS 5.4).
 * Loaded by the KernelPatch runtime used by APatch or KPatch-Next, NOT insmod
 * — so it also works where module signing blocks a `.ko`.
 *
 * STATUS: builds (`make kpm`) and runs end-to-end under QEMU via the KPM
 * harness (../test/run-kpm.sh). CI boots representative images for every
 * offset table and checks all enumeration and optional filesystem vectors;
 * socket-bind denial adds a state-level check that verifies the socket stayed
 * unbound, not just errno.
 * Runtime config/status/stats use KernelPatch's ctl0 supercall; the QEMU A/B
 * harness passes the same control-v2 snapshot as load args for headless
 * bring-up. Every per-kver
 * offset must pass the harness before that version ships — a wrong offset can
 * become a contained QEMU panic / A/B failure here or a bootloop on a device.
 *
 * DESIGN:
 *   - One source plus a runtime kver offset table (kver_offsets.h), producing
 *     one binary across the supported Android kernel families from 4.9
 *     through 6.12 (the exact minor families are listed in the README).
 *   - Per-call state lives in `fargs->local.dataN`, so a task migrating CPUs
 *     between the before/after callbacks cannot mix state with another call.
 *   - Filtering algorithms shared with the `.ko` via ../shared/vpnhide_logic.h.
 *   - VPN-name matching from the generated single source of truth
 *     (../generated/iface_lists.h → data/interfaces.toml), incl. the `if<N>`
 *     pattern (issue #86).
 *   - rt_fill_info (single-route lookup) is intentionally not hooked because
 *     its argument/register mapping is not stable across tested kernels.
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
KPM_VERSION(VPNHIDE_KPM_BUILD_VERSION);
KPM_LICENSE("GPL v2"); /* GPL to use GPL-only kernel symbols at runtime */
KPM_AUTHOR("okhsunrog");
KPM_DESCRIPTION("Hide VPN interfaces from selected UIDs (KPM backend, beta)");

#define MODNAME "vpnhide"
/* Mirror of vpnhide_protocol::MAX_TARGET_UIDS (crates/protocol/src/lib.rs); the
 * activator truncates the projected config to this many targets, so keep both in
 * sync. */
#define MAX_TARGET_UIDS 160

/* ------------------------------------------------------------------ */
/*  Resolved state                                                    */
/* ------------------------------------------------------------------ */

static const struct vpnhide_offsets *off; /* selected per running kver */
static const struct vpnhide_vfs_offsets *vfs_off;

/* Live config (protocol §4.3). Each target carries a per-hook mask so the app
 * can enable hooks individually; a hook fires only if its bit is set for the
 * calling uid.
 *
 * Guarded by a seqlock so a hook reader (every hooked syscall, on any CPU) never
 * sees a half-applied target set. A writer (rare, root-initiated: ctl0 config or
 * an initial control-v2 snapshot) bumps cfg_seq odd, mutates targets[] in place, then
 * bumps it even; readers snapshot under matching even-seq reads and retry on a
 * concurrent write. KP has no kernel spinlock/RCU, and a double-buffer (two
 * copies of targets[]) grew the .kpm enough to break KP boot on the 6.12 image —
 * the seqlock costs one extra word instead. ctl0 calls can arrive concurrently
 * (for example, boot activation racing an app-triggered reconcile), so a small
 * atomic writer gate serializes mutations before cfg_seq is made odd. A
 * contending ctl0 returns busy rather than spinning in preemptible kernel
 * context; the userspace activator retries the rare collision. */
/* Parallel arrays, not an array of structs: the hot-path lookup touches only
 * the uids, so keeping them contiguous halves the cache lines a search walks
 * (a full 160-uid set is one 640-byte run). The parser hands them over sorted
 * ascending (protocol §4.3), which is what lets the search be a bisection. */
static uint32_t target_uids[MAX_TARGET_UIDS];
static uint32_t target_masks[MAX_TARGET_UIDS];
/* KPM has no allocator API suitable for ctl0 parsing. Keep the enlarged
 * snapshot in static storage, serialized independently from the live-config
 * seqlock so hook readers never spin while userspace text is being parsed. */
static struct vpnhide_target config_staging[MAX_TARGET_UIDS];
static volatile uint32_t config_staging_writer;
static int nr_targets;
/* Hookmask for any uid NOT in target_uids (protocol §4.3 `default`). Zero — the
 * shipped blacklist — makes the array the set to act on; non-zero inverts that
 * and makes it the exception list. */
static uint32_t default_hookmask;
static uint32_t active_hook_mask;
/* Locked at module load: these globally hot hooks cannot be added or removed by
 * a later ctl0 config. The canonical optionalFeatures setting is projected
 * into KPM load args by the activator. */
static bool filesystem_hiding_requested;

/*
 * First uid Android hands to an ordinary app; everything below is a system AID
 * (system_server 1000, radio 1001, network_stack 1073, shell 2000, the OEM
 * 5000s). Compared on the app-id so a uid from a secondary profile — 1010234 in
 * profile 10 — classifies the same as 10234 in the owner profile.
 */
#define VPNHIDE_FIRST_APP_UID 10000
#define VPNHIDE_PER_USER_RANGE 100000
static volatile uint32_t
	cfg_seq; /* even = stable, odd = a writer is mid-update */
static volatile uint32_t cfg_writer; /* 0 = free, 1 = config writer active */
static bool debug_enabled;
#define VPNHIDE_CFG_READ_ATTEMPTS 4

/* status (protocol §4.3/§5.1): which hooks actually installed, and the dominant
 * fault code. Filled at init, read back via ctl0 `status`. */
static uint32_t installed_hooks;
static uint32_t last_error;

/* Exact registrations owned by this KPM. Teardown must unwrap the same
 * compiler clone that install_hook resolved, and only after hook_wrap actually
 * succeeded; resolving names again during exit cannot guarantee either. */
#define VPNHIDE_MAX_HOOK_REGISTRATIONS 16
struct vpnhide_hook_registration {
	void *function;
	void *before;
	void *after;
};
static struct vpnhide_hook_registration
	hook_registrations[VPNHIDE_MAX_HOOK_REGISTRATIONS];
static unsigned int nr_hook_registrations;

/* Native interception stats, cumulative since KPM load. The dense KPM mapping
 * stores the shared kernel hooks plus its optional filesystem hook; the global
 * id space also contains LSPosed/Zygisk hooks that can never fire here. A zero UID is the empty-slot
 * sentinel (system AIDs are unconditionally excluded), claimed atomically by
 * open addressing so concurrent first hits for one UID converge on one slot. */
static uint32_t stats_uids[MAX_TARGET_UIDS];
static unsigned long long stats_counts[MAX_TARGET_UIDS]
				      [VPNHIDE_KPM_STATS_HOOK_COUNT];
/* KernelPatch and both userspace clients cap one ctl0 reply at 4096 bytes.
 * Stats are cursor-paged by UID; status remains a small single reply. */
#define VPNHIDE_OUT_MAX 4096
#define VPNHIDE_STATS_ROW_MAX 512
#define VPNHIDE_STATS_TRAILER_RESERVE 64

/* Kernel functions resolved at init via kallsyms. */
static unsigned long (*_copy_from_user)(void *, const void *, unsigned long);
static unsigned long (*_copy_to_user)(void *, const void *, unsigned long);
static uint64_t (*_read_sanitised_ftr_reg)(uint32_t);
static void (*_skb_trim)(void *, unsigned int);
static int (*_netdev_get_name)(void *, char *, int);
static char *(*_dentry_path_raw)(void *, char *, int);
static int (*_vfs_statfs)(const void *, void *);
static void (*_path_put)(const void *);
static void (*_fput)(void *);

/* Linux sys_reg(3, 0, 0, 7, 1); ID_AA64MMFR1_EL1.PAN is bits [23:20]. */
#define VPNHIDE_SYS_ID_AA64MMFR1_EL1 0x180720u
#define VPNHIDE_ID_AA64MMFR1_PAN_SHIFT 20u

#define vpnhide_dbg(fmt, ...)                                          \
	do {                                                           \
		if (__atomic_load_n(&debug_enabled, __ATOMIC_RELAXED)) \
			logki(MODNAME ": " fmt, ##__VA_ARGS__);        \
	} while (0)

/* ------------------------------------------------------------------ */
/*  Core helpers                                                      */
/* ------------------------------------------------------------------ */

/* Recompute the OR of every target's hookmask and the default (the fast-path
 * gate). Called by a writer while holding the seqlock (cfg_seq odd), reading
 * the live arrays. The default has to be folded in: with a non-zero default a
 * hook can be live for uids that appear in no target record at all. */
static uint32_t compute_active_hook_mask(int count)
{
	uint32_t mask = __atomic_load_n(&default_hookmask, __ATOMIC_RELAXED);
	int i;

	for (i = 0; i < count; i++)
		mask |= __atomic_load_n(&target_masks[i], __ATOMIC_RELAXED);
	return mask;
}

/* Seqlock write side. Claim the writer gate, then bump cfg_seq odd before
 * mutating targets[]/nr_targets/active_hook_mask in place and even after. Do
 * not spin here: if a lock holder were preempted by another ctl0 caller on the
 * same CPU, a home-grown spinlock without preempt_disable could deadlock. */
static int cfg_try_write_begin(void)
{
	uint32_t expected = 0;

	if (!__atomic_compare_exchange_n(&cfg_writer, &expected, 1, false,
					 __ATOMIC_ACQUIRE, __ATOMIC_RELAXED))
		return 0;
	/* Acquire ordering keeps the protected writes below from moving before
	 * the odd sequence value becomes visible to readers. */
	__atomic_fetch_add(&cfg_seq, 1, __ATOMIC_ACQ_REL);
	return 1;
}

static void cfg_write_end(void)
{
	__atomic_fetch_add(&cfg_seq, 1, __ATOMIC_RELEASE);
	__atomic_store_n(&cfg_writer, 0, __ATOMIC_RELEASE);
}

/* The parser needs a full target snapshot, but putting 160 entries on the KPM
 * stack would consume 1280 bytes before ctl0's other locals. Serialize use of
 * the static scratch without making cfg_seq odd: readers keep running while
 * parsing, and only the short copy into live arrays takes the seqlock. */
static int config_staging_try_claim(void)
{
	uint32_t expected = 0;

	return __atomic_compare_exchange_n(&config_staging_writer, &expected, 1,
					   false, __ATOMIC_ACQUIRE,
					   __ATOMIC_RELAXED);
}

static void config_staging_release(void)
{
	__atomic_store_n(&config_staging_writer, 0, __ATOMIC_RELEASE);
}

/* True if `hook_id` is enabled for the calling uid (per-hook gate, §4.3).
 * Seqlock read side: accept only matching even sequence reads, so the mask gate
 * and per-uid scan come from one consistent snapshot. Retries are bounded: a
 * hook callback can preempt the ctl0 writer on the same CPU, where spinning
 * until that writer resumes would deadlock. A call that collides with the very
 * short config copy therefore passes through unchanged. */
static int hook_active(enum vpnhide_hook_id hook_id)
{
	uid_t uid = current_uid();
	uint32_t s1, s2;
	uint32_t bit = vpnhide_hook_bit(hook_id);
	int attempt;

	/* Below the app range a uid is not an app, it is a platform identity
	 * shared by many components: an app declaring sharedUserId
	 * "android.uid.system" resolves to 1000, the same uid as system_server.
	 * Since UID is the targeting key (§4.3), "hide from that app" is not
	 * expressible there — the nearest thing the wire can say is "hide from
	 * everything running as 1000", which is how a device ends up believing
	 * it has no route. NOT the same set as FLAG_SYSTEM: vendor-preinstalled
	 * apps keep ordinary 10xxx uids and stay targetable.
	 *
	 * Unconditional and deliberately not expressible in a config — neither
	 * a target nor a `default` lifts it. Also keeps uid 0 honest, which the
	 * app's root-differential diagnostics use as ground truth.
	 */
	if (uid % VPNHIDE_PER_USER_RANGE < VPNHIDE_FIRST_APP_UID)
		return 0;

	for (attempt = 0; attempt < VPNHIDE_CFG_READ_ATTEMPTS; attempt++) {
		int result = 0;

		s1 = __atomic_load_n(&cfg_seq, __ATOMIC_ACQUIRE);
		if (s1 & 1u)
			continue; /* a writer is mid-update */
		if (__atomic_load_n(&active_hook_mask, __ATOMIC_RELAXED) &
		    bit) {
			int lo = 0;
			int hi =
				__atomic_load_n(&nr_targets, __ATOMIC_RELAXED) -
				1;
			uint32_t mask = __atomic_load_n(&default_hookmask,
							__ATOMIC_RELAXED);

			/* Sorted ascending by the parser (§4.3), so this is a
			 * bisection rather than a walk — it runs on every
			 * hooked call, unlike the parse that ordered it. */
			while (lo <= hi) {
				int mid = lo + (hi - lo) / 2;

				uint32_t target_uid = __atomic_load_n(
					&target_uids[mid], __ATOMIC_RELAXED);

				if (target_uid == uid) {
					mask = __atomic_load_n(
						&target_masks[mid],
						__ATOMIC_RELAXED);
					break;
				}
				if (target_uid < uid)
					lo = mid + 1;
				else
					hi = mid - 1;
			}
			result = (mask & bit) != 0;
		}
		s2 = __atomic_load_n(&cfg_seq, __ATOMIC_ACQUIRE);
		if (s1 == s2)
			return result;
		/* s1 was even; retry if a write started or finished. */
	}
	return 0;
}

static int stats_slot_for_uid(uint32_t uid)
{
	uint32_t first;
	int probe;

	if (!uid)
		return -1;
	first = (uid * 2654435761u) % MAX_TARGET_UIDS;
	for (probe = 0; probe < MAX_TARGET_UIDS; probe++) {
		int i = (int)((first + (uint32_t)probe) % MAX_TARGET_UIDS);
		uint32_t owner =
			__atomic_load_n(&stats_uids[i], __ATOMIC_ACQUIRE);

		if (owner == uid)
			return i;
		if (owner != 0)
			continue;
		if (__sync_bool_compare_and_swap(&stats_uids[i], 0, uid))
			return i;
		/* A competing first hit claimed this deterministic probe position.
		 * Re-read it before advancing: if it was the same UID, returning this
		 * slot prevents the duplicate-row race of the old two-pass scan. */
		if (__atomic_load_n(&stats_uids[i], __ATOMIC_ACQUIRE) == uid)
			return i;
	}

	return -1;
}

static void record_hook_hit(enum vpnhide_hook_id hook_id)
{
	int hook_slot, uid_slot;

	hook_slot = vpnhide_kpm_stats_hook_slot(hook_id);
	if (hook_slot < 0)
		return;
	uid_slot = stats_slot_for_uid((uint32_t)current_uid());
	if (uid_slot >= 0)
		__sync_fetch_and_add(&stats_counts[uid_slot][hook_slot], 1ULL);
}

static int next_stats_uid(uint32_t after, uint32_t *uid_out, int *slot_out)
{
	uint32_t best = 0xffffffffu;
	int best_slot = -1, i;

	for (i = 0; i < MAX_TARGET_UIDS; i++) {
		uint32_t uid =
			__atomic_load_n(&stats_uids[i], __ATOMIC_ACQUIRE);

		if (uid > after && (best_slot < 0 || uid < best)) {
			best = uid;
			best_slot = i;
		}
	}
	if (best_slot < 0)
		return 0;
	*uid_out = best;
	*slot_out = best_slot;
	return 1;
}

static unsigned long format_stats_row(char *buf, unsigned long cap,
				      uint32_t uid, int uid_slot)
{
	struct vpnhide_buf b = { .p = buf, .cap = cap, .len = 0 };
	int hook, have_count = 0;

	vpnhide_put_hex(&b, uid);
	for (hook = 0; hook < VPNHIDE_KPM_STATS_HOOK_COUNT; hook++) {
		unsigned long long count = __atomic_load_n(
			&stats_counts[uid_slot][hook], __ATOMIC_RELAXED);

		if (!count)
			continue;
		have_count = 1;
		vpnhide_putc(&b, ' ');
		vpnhide_put_hex(&b, vpnhide_kpm_stats_hook_id(hook));
		vpnhide_putc(&b, ':');
		vpnhide_put_hex(&b, count);
	}
	if (!have_count)
		return 0;
	vpnhide_putc(&b, '\n');
	return b.len;
}

/* A page is ordered by UID, independent of the hash-table layout. `page`
 * carries the requested cursor, the next cursor (or `done`), and emitted UID
 * row count. A non-final page deliberately omits its final newline: an old
 * activator therefore treats it as truncated and refuses partial totals, while
 * a pagination-aware activator validates the complete trailer and continues. */
static unsigned long format_stats_page(char *buf, unsigned long cap,
				       uint32_t requested)
{
	struct vpnhide_buf b = { .p = buf, .cap = cap, .len = 0 };
	char row[VPNHIDE_STATS_ROW_MAX];
	uint32_t cursor = requested, uid;
	unsigned int rows = 0;
	int slot, more;

	if (cap < VPNHIDE_STATS_ROW_MAX + VPNHIDE_STATS_TRAILER_RESERVE)
		return 0;
	vpnhide_put_header(&b, "stats", VPNHIDE_TELEMETRY_VERSION);
	while (next_stats_uid(cursor, &uid, &slot)) {
		unsigned long row_len =
			format_stats_row(row, sizeof(row), uid, slot);

		if (!row_len) {
			cursor = uid;
			continue;
		}
		if (b.len + row_len + VPNHIDE_STATS_TRAILER_RESERVE > cap)
			break;
		for (unsigned long i = 0; i < row_len; i++)
			vpnhide_putc(&b, row[i]);
		cursor = uid;
		rows++;
	}
	more = next_stats_uid(cursor, &uid, &slot);
	vpnhide_puts(&b, "page ");
	vpnhide_put_hex(&b, requested);
	vpnhide_putc(&b, ' ');
	if (more)
		vpnhide_put_hex(&b, cursor);
	else
		vpnhide_puts(&b, "done");
	vpnhide_putc(&b, ' ');
	vpnhide_put_hex(&b, rows);
	if (!more)
		vpnhide_putc(&b, '\n');
	return b.len;
}

static long copy_ctl_reply(char __user *out_msg, const char *buf,
			   unsigned long len)
{
	if (!len)
		return 0;
	if (!out_msg || !_copy_to_user || _copy_to_user(out_msg, buf, len))
		return -1;
	return (long)len;
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

/* Read net_device.name at the selected table's version-specific offset.
 *
 * `dev` is resolved through version-specific struct offsets (dev_from_fib_info,
 * dev_from_fib6_info, deref2). On a vendor kernel whose layout differs from the
 * offset table, that resolution can yield a bogus value; reject anything that
 * is not a kernel-space pointer so a wrong table degrades to "not hidden"
 * instead of dereferencing garbage and panicking. ptr_is_kernel is defined
 * further down but forward-declared for this early use. */
static int ptr_is_kernel(const void *p);
static const char *netdev_name(void *dev)
{
	if (!ptr_is_kernel(dev))
		return 0;
	return (const char *)((char *)dev + off->netdev_name);
}

/* Every hook_fargsN type shares this hook_fargs0_t prefix. Keep the common skb
 * rollback state machine here so all netlink hooks preserve identical error,
 * trim, return-value, and statistics semantics. */
static void reset_skb_filter(void *raw_fargs)
{
	hook_fargs0_t *fargs = raw_fargs;

	fargs->local.data0 = 0;
}

static void mark_skb_for_filter(void *raw_fargs, void *skb)
{
	hook_fargs0_t *fargs = raw_fargs;

	fargs->local.data0 = 1;
	fargs->local.data1 = (uint64_t)skb;
	fargs->local.data2 =
		(uint64_t) * (unsigned int *)((char *)skb + off->skb_len);
}

static void finish_skb_filter(void *raw_fargs, enum vpnhide_hook_id hook_id)
{
	hook_fargs0_t *fargs = raw_fargs;

	if (!fargs->local.data0 || (long)fargs->ret < 0)
		return;
	_skb_trim((void *)fargs->local.data1, (unsigned int)fargs->local.data2);
	fargs->ret = 0;
	record_hook_hit(hook_id);
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
static int kpm_is_public_host_route4(hook_fargs12_t *fargs, const void *p,
				     void *dev)
{
	const unsigned char *dst_be;
	uint32_t dst_val = 0;
	int dst_len;

	if (!dev)
		return 0;
	if (off->fib_dump_fi_via_fri) {
		/* 5.6+: p is a fib_rt_info* with __be32 dst @+12, int dst_len
		 * @+16 (constant across GKI 5.10..6.12). */
		dst_len = *(const int *)((const char *)p + 16);
		dst_be = (const unsigned char *)p + 12;
	} else {
		/* <5.6 (5.4/4.19/4.14): fib_dump_info(skb,...,tb_id,type,dst,
		 * dst_len,...) passes the __be32 dst by value in arg 6 and
		 * dst_len in arg 7. The register holds the network-order value,
		 * so the in-memory bytes of a local copy are the octets a.b.c.d. */
		dst_val = (uint32_t)fargs->args[6];
		dst_len = (int)fargs->args[7];
		dst_be = (const unsigned char *)&dst_val;
	}
	if (dst_len != 32)
		return 0;
	if (!vpnhide_is_public_ipv4(dst_be))
		return 0;
	return vpnhide_iface_is_physical(netdev_name(dev));
}

/* IPv6 analogue of kpm_is_public_host_route4: a public /128 host-route pinned to
 * a physical uplink (the route a VPN client installs to reach the server, which
 * leaks its IPv6 even when the tun is hidden — the .ko's
 * is_public_host_route6_via_physical). The route's destination is a rt6key
 * { in6_addr addr@0; int plen@16 } whose offset depends on the kernel's IPv6
 * route model: in fib6_info at off->fib6_info_fib6_dst (5.x/6.x), or in the
 * embedded rt6_info at off->rt6_dst on the pre-fib6_info rt6_via_dst path
 * (4.9/4.14).
 * A 0 offset for the active model disables the check. `rt` is the route arg to
 * rt6_fill_node; the rt6key sits within the struct dev_from_fib6_info already
 * reads, so it is in-bounds. Address/iface logic shared with the .ko. */
static int kpm_is_public_host_route6(void *rt, void *dev)
{
	const unsigned char *dst;
	unsigned int dst_off;
	int plen;

	if (!rt || !dev)
		return 0;
	dst_off = off->rt6_via_dst ? off->rt6_dst : off->fib6_info_fib6_dst;
	if (!dst_off)
		return 0;
	dst = (const unsigned char *)rt + dst_off;
	plen = *(const int *)(dst + 16); /* rt6key.plen */
	if (plen != 128)
		return 0;
	if (!vpnhide_is_public_ipv6(dst)) /* rt6key.addr @ +0 */
		return 0;
	return vpnhide_iface_is_physical(netdev_name(dev));
}

/* ================================================================== */
/*  Hook 1: fib_route_seq_show — /proc/net/route                      */
/*  arg0 = struct seq_file *.  Compact VPN lines out of this call's    */
/*  newly-written region using the shared seq-line compactor.          */
/* ================================================================== */

static void fib_route_before(hook_fargs2_t *fargs, void *udata)
{
	/* Stash seq_file->count at entry so the after-callback only touches this
	 * call's output, not entries already present in the buffer. */
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
/*  Hook 2: rtnl_fill_ifinfo — RTM_NEWLINK (getifaddrs path)          */
/*  arg0 = skb, arg1 = net_device.  If the dev is a VPN iface and the  */
/*  caller is a target, undo whatever the fill wrote (skb_trim back to  */
/*  the saved length) and return 0 — same approach as the .ko.         */
/*  We do NOT return -EMSGSIZE (infinite retry on 6.1 — issue #38).    */
/* ================================================================== */

static void rtnl_fill_before(hook_fargs12_t *fargs, void *udata)
{
	void *skb = (void *)fargs->arg0;
	void *dev = (void *)fargs->arg1;

	reset_skb_filter(fargs);
	if (!hook_active(VPNHIDE_HOOK_RTNL_FILL_IFINFO) || !skb || !dev)
		return;
	if (!iface_is_vpn(netdev_name(dev)))
		return;

	mark_skb_for_filter(fargs, skb);
}

static void rtnl_fill_after(hook_fargs12_t *fargs, void *udata)
{
	finish_skb_filter(fargs, VPNHIDE_HOOK_RTNL_FILL_IFINFO);
}

/* ================================================================== */
/*  Hook 4: dev_ioctl — per-interface ioctls (SIOCGIF* by name)        */
/*  arg1 = cmd, arg2 = ifr (ifr_name at offset 0, uapi-stable). The arg */
/*  can be a kernel struct ifreq* or a userspace pointer: the transition */
/*  was backported differently across Android 4.x vendor/common trees.  */
/*  Dereferencing a user ptr from kernel context faults under PAN, so   */
/*  detect its address domain and use the matching read path. If it      */
/*  names a VPN iface, return -ENODEV.                                  */
/* ================================================================== */

#define VPNHIDE_ENODEV ((uint64_t)(-19))

/* arm64: TTBR1 (kernel) addresses have the top 16 bits set; user ptrs don't. */
static int ptr_is_kernel(const void *p)
{
	return ((unsigned long)p >> 48) == 0xffffUL;
}

/* ================================================================== */
/*  Hook 11: socket interface binding                                 */
/*                                                                    */
/*  A target must not be able to use SO_BINDTODEVICE or               */
/*  SO_BINDTOIFINDEX as an oracle for a hidden VPN interface. KPM can */
/*  refuse before the original function mutates sk_bound_dev_if: set  */
/*  skip_origin and return ENODEV.                                    */
/*                                                                    */
/*  5.9+ passes optval as the two-register sockptr_t aggregate. We    */
/*  copy it once into hook_fargs.local and rewrite the aggregate to a  */
/*  KERNEL_SOCKPTR pointing at that per-call snapshot. Thus a second   */
/*  userspace thread cannot swap a checked physical name/index for a  */
/*  VPN one between this callback and the kernel's own copy. Before   */
/*  5.7 the first bind already requires CAP_NET_RAW, so KPM leaves it */
/*  entirely to the native path and preserves its exact errno. On     */
/*  5.7-5.8 a deeper *_bindto*_locked hook sees the resolved ifindex  */
/*  after the user copy but still before the socket mutation.          */
/* ================================================================== */

#define VPNHIDE_SO_BINDTODEVICE 25u
#define VPNHIDE_SO_BINDTOIFINDEX 62u

static int sockopt_uses_sockptr(void)
{
	return (unsigned int)kver >= VPNHIDE_KVER(5, 9, 0);
}

static int sockopt_takes_sk(void)
{
	return (unsigned int)kver >= VPNHIDE_KVER(6, 1, 0);
}

static int socket_bind_uses_index_hook(void)
{
	return (unsigned int)kver >= VPNHIDE_KVER(5, 7, 0) &&
	       (unsigned int)kver < VPNHIDE_KVER(5, 9, 0);
}

static const char *socket_bind_index_hook_name(void)
{
	return (unsigned int)kver < VPNHIDE_KVER(5, 8, 0) ?
		       "sock_setbindtodevice_locked" :
		       "sock_bindtoindex_locked";
}

static int copy_sockopt_bytes(hook_fargs8_t *fargs, void *dst, unsigned int len)
{
	const unsigned char *src = (const unsigned char *)fargs->arg3;
	unsigned char *out = (unsigned char *)dst;
	unsigned int i;

	if (!src)
		return -1;
	/* sockptr_t.is_kernel is the second eightbyte (arg4). It is trustworthy
	 * because the syscall wrapper, not userspace, constructs sockptr_t. Never
	 * infer this from the numeric pointer value: an app may pass a 0xffff...
	 * address deliberately, and directly dereferencing it would panic instead
	 * of producing the native EFAULT. This wrapper is used only on 5.9+. */
	if (sockopt_uses_sockptr() && fargs->arg4) {
		for (i = 0; i < len; i++)
			out[i] = src[i];
		return 0;
	}
	if (!_copy_from_user || _copy_from_user(dst, src, len) != 0)
		return -1;
	return 0;
}

static unsigned int sockopt_len(const hook_fargs8_t *fargs)
{
	return (unsigned int)(sockopt_uses_sockptr() ? fargs->arg5 :
						       fargs->arg4);
}

static void *sockopt_sk(const hook_fargs8_t *fargs, int takes_sk)
{
	void *sock;

	if (takes_sk)
		return (void *)fargs->arg0;
	sock = (void *)fargs->arg0;
	return sock ? *(void **)((char *)sock + off->socket_sk) : 0;
}

static void freeze_sockptr(hook_fargs8_t *fargs, void *snapshot)
{
	if (!sockopt_uses_sockptr())
		return;
	/* sockptr_t { pointer, is_kernel:1 } occupies x3+x4 on arm64. */
	fargs->arg3 = (uint64_t)snapshot;
	fargs->arg4 = 1;
}

static void deny_socket_bind(void *raw_fargs)
{
	hook_fargs0_t *fargs = (hook_fargs0_t *)raw_fargs;

	fargs->ret = VPNHIDE_ENODEV;
	fargs->skip_origin = 1;
	record_hook_hit(VPNHIDE_HOOK_SOCKET_BIND_INTERFACE);
}

/* Return true for a hidden or currently unknown positive index. Linux accepts
 * an unknown positive index and leaves the socket waiting for a future device,
 * so allowing it would bypass a name/index check performed only once. */
static int socket_bind_ifindex_hidden(void *sk, int ifindex)
{
	void *net = sk ? *(void **)((char *)sk + off->sock_net) : 0;
	char name[VPNHIDE_IFNAMSIZ];
	unsigned int i;

	if (ifindex <= 0)
		return 0;
	if (!net || !_netdev_get_name)
		return 1;
	for (i = 0; i < VPNHIDE_IFNAMSIZ; i++)
		name[i] = 0;
	return _netdev_get_name(net, name, ifindex) != 0 || iface_is_vpn(name);
}

static void socket_bind_index_before(hook_fargs4_t *fargs, void *udata)
{
	int ifindex = (int)fargs->arg1;

	if (!hook_active(VPNHIDE_HOOK_SOCKET_BIND_INTERFACE))
		return;
	if (socket_bind_ifindex_hidden((void *)fargs->arg0, ifindex))
		deny_socket_bind(fargs);
}

static void socket_bind_before_common(hook_fargs8_t *fargs, int takes_sk)
{
	unsigned int optname = (unsigned int)fargs->arg2;
	unsigned int optlen;
	unsigned char *snapshot = (unsigned char *)&fargs->local;
	unsigned int i;

	/* No `level` gate on purpose. Reaching sock_setsockopt / sk_setsockopt
	 * proves level == SOL_SOCKET by control flow, and sk_setsockopt never reads
	 * its `level` parameter — so whole-kernel LTO elides setting it at the direct
	 * __sys_setsockopt -> sk_setsockopt call, and fargs->arg1 is then garbage
	 * (observed 0). A `arg1 != SOL_SOCKET` check would wrongly pass the bind
	 * through and leak on those builds (the same defect fixed in the .ko). Gate
	 * on the hook toggle alone. */
	if (!hook_active(VPNHIDE_HOOK_SOCKET_BIND_INTERFACE))
		return;
	if (optname != VPNHIDE_SO_BINDTODEVICE &&
	    optname != VPNHIDE_SO_BINDTOIFINDEX)
		return;
	/* Before 5.7 the native capability gate owns this vector. On 5.7-5.8
	 * the resolved-index hook owns it so the old user-pointer ABI cannot
	 * introduce a check/use race. This wrapper is authoritative only once
	 * sockptr_t can carry our immutable kernel snapshot. */
	if (!sockopt_uses_sockptr())
		return;

	optlen = sockopt_len(fargs);
	if (optname == VPNHIDE_SO_BINDTODEVICE) {
		unsigned int copy_len = optlen;

		/* The downstream helper takes int and returns EINVAL first. */
		if ((int)optlen < 0)
			return;
		if (copy_len > VPNHIDE_IFNAMSIZ - 1)
			copy_len = VPNHIDE_IFNAMSIZ - 1;
		for (i = 0; i < VPNHIDE_IFNAMSIZ; i++)
			snapshot[i] = 0;
		if (copy_len && copy_sockopt_bytes(fargs, snapshot, copy_len))
			return; /* let the original produce EFAULT */
		freeze_sockptr(fargs, snapshot);
		if (snapshot[0] && iface_is_vpn((const char *)snapshot))
			deny_socket_bind(fargs);
		return;
	}

	if (optlen < sizeof(int))
		return;
	if (copy_sockopt_bytes(fargs, snapshot, sizeof(int)))
		return;
	freeze_sockptr(fargs, snapshot);
	{
		int ifindex = *(int *)snapshot;
		void *sk = sockopt_sk(fargs, takes_sk);

		if (socket_bind_ifindex_hidden(sk, ifindex))
			deny_socket_bind(fargs);
	}
}

static void socket_bind_sock_before(hook_fargs8_t *fargs, void *udata)
{
	socket_bind_before_common(fargs, 0);
}

static void socket_bind_sk_before(hook_fargs8_t *fargs, void *udata)
{
	socket_bind_before_common(fargs, 1);
}

/* The get-interface-by-name opcode range (0x8910..0x8970) is the shared
 * predicate vpnhide_ioctl_is_get_by_name() in shared/vpnhide_logic.h — one
 * source of truth so this gate cannot narrow below SIOCGIFINDEX (0x8933) again
 * while the zygisk copy stays wide, which is exactly how a leak crept in before. */
static void dev_ioctl_after(hook_fargs5_t *fargs, void *udata)
{
	unsigned long cmd = (unsigned long)fargs->arg1;
	const char *ifr = (const char *)fargs->arg2; /* ifr_name @ offset 0 */
	char name[VPNHIDE_IFNAMSIZ];
	int is_vpn;

	if ((long)fargs->ret != 0 || !ifr)
		return;
	if (!hook_active(VPNHIDE_HOOK_DEV_IOCTL) ||
	    !vpnhide_ioctl_is_get_by_name(cmd))
		return;

	if (ptr_is_kernel(ifr)) {
		is_vpn = iface_is_vpn(ifr); /* >=5.5: ifr is kernel memory */
	} else {
		/* Older ABI: ifr is a __user pointer — copy the name safely. */
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
	char e[VPNHIDE_IFREQ_SZ];
	char zero[VPNHIDE_IFREQ_SZ] = { 0 };
	char *req;
	int len, n, i, dst = 0;

	uint32_t req_lo = 0, req_hi = 0;
	long rc_len, rc_lo, rc_hi;

	if (!uifc || !_copy_from_user || !_copy_to_user)
		return 0;
	/* Read the UAPI pointer as two fixed-width words. The 5.10/5.15 KPM QEMU
	 * targets preserved only its low word when it was copied directly into a
	 * pointer-typed local; reconstructing both halves keeps the arm64 address. */
	rc_len = _copy_from_user(&len, uifc, sizeof(len));
	rc_lo = _copy_from_user(&req_lo, (char *)uifc + 8, sizeof(req_lo));
	rc_hi = _copy_from_user(&req_hi, (char *)uifc + 12, sizeof(req_hi));
	req = (char *)(unsigned long)(((uint64_t)req_hi << 32) | req_lo);
	if (rc_len || rc_lo || rc_hi)
		return 0;
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
		int tail;

		/* The shortened length does not make the old entries disappear from
		 * the caller-owned buffer. Clear only the kernel-written slots removed
		 * by compaction, leaving unused caller capacity untouched. */
		for (tail = dst; tail < n; tail++) {
			if (_copy_to_user(req + (long)tail * VPNHIDE_IFREQ_SZ,
					  zero, VPNHIDE_IFREQ_SZ))
				return 0;
		}

		if (_copy_to_user(uifc, &newlen, sizeof(newlen)))
			return 0; /* failed to shrink ifc_len */
		vpnhide_dbg("sock_ioctl: ifconf %d -> %d\n", len, newlen);
		return 1;
	}
	return 0;
}

static void sock_ioctl_before(hook_fargs3_t *fargs, void *udata)
{
	unsigned long cmd = (unsigned long)fargs->arg1;

	fargs->local.data0 = 0;
	if (cmd != VPNHIDE_SIOCGIFCONF || !hook_active(VPNHIDE_HOOK_SOCK_IOCTL))
		return;
	fargs->local.data0 = fargs->arg2;
}

static void sock_ioctl_after(hook_fargs3_t *fargs, void *udata)
{
	void *argp = (void *)fargs->local.data0;

	if ((long)fargs->ret != 0 || !argp)
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

/* p = *(*(base+off1)+off2) with NULL guards (two-pointer deref). The
 * intermediate pointer is validated as kernel-space before the second deref:
 * if off1 is wrong on a vendor kernel, *(base+off1) is a bogus value, and
 * dereferencing it would fault — bail instead. */
static void *deref2(void *base, unsigned int off1, unsigned int off2)
{
	void *p;

	if (!base)
		return 0;
	p = *(void **)((char *)base + off1);
	if (!ptr_is_kernel(p))
		return 0;
	return *(void **)((char *)p + off2);
}

/* Shared by both addr-fill hooks: stash skb + len if ifa's dev is VPN. The
 * caller passes its own hook id so the per-hook gate (§4.3) is per-hook even
 * though the body is shared. */
static void addr_fill_before(hook_fargs4_t *fargs, void *dev,
			     enum vpnhide_hook_id hook_id)
{
	void *skb = (void *)fargs->arg0;

	reset_skb_filter(fargs);
	if (!hook_active(hook_id) || !skb || !dev)
		return;
	if (!iface_is_vpn(netdev_name(dev)))
		return;
	mark_skb_for_filter(fargs, skb);
}

static void addr_fill_after_hook(hook_fargs4_t *fargs,
				 enum vpnhide_hook_id hook_id)
{
	finish_skb_filter(fargs, hook_id);
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

	reset_skb_filter(fargs);
	if (!hook_active(VPNHIDE_HOOK_FIB_DUMP_INFO) || !skb || !p)
		return;
	fi = off->fib_dump_fi_via_fri ? *(void **)p : p; /* fib_rt_info.fi @0 */
	dev = dev_from_fib_info(fi);
	if (!dev)
		return;
	/* Hide the route if its output dev is a VPN iface, OR it is a public /32
	 * host-route pinned to a physical uplink — the .ko hides both, so the KPM
	 * must too for backend parity. Works on both the 5.6+ fib_rt_info ABI and
	 * the <5.6 (5.4/4.19/4.14) dst/dst_len-as-args ABI. */
	if (!iface_is_vpn(netdev_name(dev)) &&
	    !kpm_is_public_host_route4(fargs, p, dev))
		return;

	mark_skb_for_filter(fargs, skb);
}

static void fib_dump_after(hook_fargs12_t *fargs, void *udata)
{
	finish_skb_filter(fargs, VPNHIDE_HOOK_FIB_DUMP_INFO);
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

	reset_skb_filter(fargs);
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

	mark_skb_for_filter(fargs, skb);
}

static void rt6_fill_after(hook_fargs12_t *fargs, void *udata)
{
	finish_skb_filter(fargs, VPNHIDE_HOOK_RT6_FILL_NODE);
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

	reset_skb_filter(fargs);
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
		mark_skb_for_filter(fargs, skb);
	}
}

static void fib_rule_after(hook_fargs8_t *fargs, void *udata)
{
	finish_skb_filter(fargs, VPNHIDE_HOOK_FIB_NL_FILL_RULE);
}

/* ================================================================== */
/*  Optional filesystem path concealment                              */
/*                                                                    */
/*  Like the .ko backend, KPM evaluates the resolved dentry rather    */
/*  than a userspace pathname, so relative openat, symlink traversal, */
/*  and bind mounts cannot bypass it. dentry_path_raw owns the rename */
/*  lock/RCU walk; vfs_statfs distinguishes sysfs/procfs without      */
/*  duplicating unstable dentry/super_block layouts in this module.   */
/* ================================================================== */

#define VPNHIDE_ENOENT ((uint64_t)(-2))
#define VPNHIDE_RESOLVED_PATH_MAX 512
#define VPNHIDE_PROC_SUPER_MAGIC 0x9fa0UL
#define VPNHIDE_SYSFS_MAGIC 0x62656572UL
#define VPNHIDE_READDIR_MARKER 0x76706e6869646572ULL

static void *file_path(void *file)
{
	return file ? (char *)file + vfs_off->file_f_path : 0;
}

static int string_equals(const char *left, const char *right)
{
	int i;

	if (!left || !right)
		return 0;
	for (i = 0; left[i] && right[i]; i++)
		if (left[i] != right[i])
			return 0;
	return left[i] == right[i];
}

static const char *string_after_prefix(const char *value, const char *prefix)
{
	int i;

	if (!value || !prefix)
		return 0;
	for (i = 0; prefix[i]; i++)
		if (value[i] != prefix[i])
			return 0;
	return value + i;
}

static int segment_equals(const char *segment, unsigned int len,
			  const char *literal)
{
	unsigned int i;

	for (i = 0; i < len && literal[i]; i++)
		if (segment[i] != literal[i])
			return 0;
	return i == len && literal[i] == '\0';
}

static int vpn_segment(const char *segment)
{
	char iface[VPNHIDE_IFNAMSIZ];
	unsigned int len = 0;

	if (!segment)
		return 0;
	while (segment[len] && segment[len] != '/') {
		if (len + 1 >= sizeof(iface))
			return 0;
		iface[len] = segment[len];
		len++;
	}
	if (!len)
		return 0;
	iface[len] = '\0';
	return iface_is_vpn(iface);
}

static int sysfs_iface_path(const char *path)
{
	const char *segment = path;
	const char *previous = 0;
	unsigned int previous_len = 0;

	if (!path || *path != '/')
		return 0;
	while (*segment) {
		const char *end;

		while (*segment == '/')
			segment++;
		if (!*segment)
			break;
		end = segment;
		while (*end && *end != '/')
			end++;
		if (previous && segment_equals(previous, previous_len, "net") &&
		    vpn_segment(segment))
			return 1;
		previous = segment;
		previous_len = (unsigned int)(end - segment);
		segment = end;
	}
	return 0;
}

static int proc_iface_path(const char *path)
{
	static const char *const prefixes[] = {
		"/sys/net/ipv4/conf/",
		"/sys/net/ipv4/neigh/",
		"/sys/net/ipv6/conf/",
		"/sys/net/ipv6/neigh/",
	};
	unsigned int i;

	for (i = 0; i < sizeof(prefixes) / sizeof(prefixes[0]); i++) {
		const char *iface = string_after_prefix(path, prefixes[i]);

		if (iface && vpn_segment(iface))
			return 1;
	}
	return 0;
}

static int proc_iface_listing(const char *path)
{
	return string_equals(path, "/sys/net/ipv4/conf") ||
	       string_equals(path, "/sys/net/ipv4/neigh") ||
	       string_equals(path, "/sys/net/ipv6/conf") ||
	       string_equals(path, "/sys/net/ipv6/neigh");
}

static int sysfs_iface_listing(const char *path)
{
	const char *last = path;
	const char *cursor;

	if (!path || *path != '/')
		return 0;
	for (cursor = path; *cursor; cursor++)
		if (*cursor == '/' && cursor[1])
			last = cursor + 1;
	return string_equals(last, "net");
}

static int error_pointer(const void *pointer)
{
	return (unsigned long)pointer >= (unsigned long)-4095;
}

static const char *resolved_dentry_path(const void *path, char *buf, int buflen)
{
	void *dentry;
	char *resolved;

	if (!path || !_dentry_path_raw)
		return 0;
	dentry = *(void **)((char *)path + vfs_off->path_dentry);
	if (!dentry)
		return 0;
	resolved = _dentry_path_raw(dentry, buf, buflen);
	return !resolved || error_pointer(resolved) ? 0 : resolved;
}

static unsigned long path_filesystem_magic(const void *path)
{
	/* struct kstatfs.f_type is its first long on every supported arm64 ABI.
	 * Keep ample opaque storage for the helper without importing kernel headers. */
	unsigned long statfs_words[32] = { 0 };

	if (!_vfs_statfs || _vfs_statfs(path, statfs_words) != 0)
		return 0;
	return statfs_words[0];
}

static int hidden_iface_path(const void *path)
{
	char buf[VPNHIDE_RESOLVED_PATH_MAX];
	const char *resolved = resolved_dentry_path(path, buf, sizeof(buf));

	if (!resolved)
		return 0;
	if (sysfs_iface_path(resolved))
		return path_filesystem_magic(path) == VPNHIDE_SYSFS_MAGIC;
	if (proc_iface_path(resolved))
		return path_filesystem_magic(path) == VPNHIDE_PROC_SUPER_MAGIC;
	return 0;
}

static int iface_listing_path(const void *path)
{
	char buf[VPNHIDE_RESOLVED_PATH_MAX];
	const char *resolved = resolved_dentry_path(path, buf, sizeof(buf));

	if (!resolved)
		return 0;
	if (sysfs_iface_listing(resolved))
		return path_filesystem_magic(path) == VPNHIDE_SYSFS_MAGIC;
	if (proc_iface_listing(resolved))
		return path_filesystem_magic(path) == VPNHIDE_PROC_SUPER_MAGIC;
	return 0;
}

static int filesystem_filter_active(void)
{
	return filesystem_hiding_requested &&
	       hook_active(VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS);
}

static void filename_lookup_after(hook_fargs5_t *fargs, void *udata)
{
	void *path = (void *)fargs->arg3;

	if ((long)fargs->ret != 0 || !filesystem_filter_active() ||
	    !hidden_iface_path(path))
		return;
	_path_put(path);
	*(uint64_t *)path = 0;
	*(uint64_t *)((char *)path + vfs_off->path_dentry) = 0;
	fargs->ret = VPNHIDE_ENOENT;
	record_hook_hit(VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS);
}

static void do_filp_open_after(hook_fargs3_t *fargs, void *udata)
{
	void *file = (void *)fargs->ret;

	if (!file || error_pointer(file) || !filesystem_filter_active() ||
	    !hidden_iface_path(file_path(file)))
		return;
	_fput(file);
	fargs->ret = VPNHIDE_ENOENT;
	record_hook_hit(VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS);
}

static void vfs_getattr_after(hook_fargs4_t *fargs, void *udata)
{
	if ((long)fargs->ret != 0 || !filesystem_filter_active() ||
	    !hidden_iface_path((void *)fargs->arg0))
		return;
	fargs->ret = VPNHIDE_ENOENT;
	record_hook_hit(VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS);
}

struct vpnhide_readdir_context {
	void *actor;
	long long pos;
	void *original_context;
	void *original_actor;
};

static int filter_dirent_name(const char *name, int namelen)
{
	char iface[VPNHIDE_IFNAMSIZ];
	int i;

	if (!name || namelen <= 0 || namelen >= (int)sizeof(iface))
		return 0;
	for (i = 0; i < namelen; i++)
		iface[i] = name[i];
	iface[namelen] = '\0';
	return iface_is_vpn(iface);
}

static int vpnhide_filldir_int(struct vpnhide_readdir_context *ctx,
			       const char *name, int namelen, long long offset,
			       uint64_t ino, unsigned int d_type)
{
	typedef int (*actor_fn)(void *, const char *, int, long long, uint64_t,
				unsigned int);
	long long *original_pos;
	int result;

	if (filter_dirent_name(name, namelen)) {
		record_hook_hit(VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS);
		return 0;
	}
	original_pos =
		(long long *)((char *)ctx->original_context + sizeof(void *));
	*original_pos = ctx->pos;
	result = ((actor_fn)ctx->original_actor)(ctx->original_context, name,
						 namelen, offset, ino, d_type);
	ctx->pos = *original_pos;
	return result;
}

static bool vpnhide_filldir_bool(struct vpnhide_readdir_context *ctx,
				 const char *name, int namelen,
				 long long offset, uint64_t ino,
				 unsigned int d_type)
{
	typedef bool (*actor_fn)(void *, const char *, int, long long, uint64_t,
				 unsigned int);
	long long *original_pos;
	bool result;

	if (filter_dirent_name(name, namelen)) {
		record_hook_hit(VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS);
		return true;
	}
	original_pos =
		(long long *)((char *)ctx->original_context + sizeof(void *));
	*original_pos = ctx->pos;
	result = ((actor_fn)ctx->original_actor)(ctx->original_context, name,
						 namelen, offset, ino, d_type);
	ctx->pos = *original_pos;
	return result;
}

static void iterate_dir_before(hook_fargs2_t *fargs, void *udata)
{
	struct vpnhide_readdir_context *wrapper =
		(struct vpnhide_readdir_context *)&fargs->local;
	void *context = (void *)fargs->arg1;

	fargs->local.data4 = 0;
	if (!context || !*(void **)context || !filesystem_filter_active() ||
	    !iface_listing_path(file_path((void *)fargs->arg0)))
		return;
	wrapper->actor = (unsigned int)kver >= VPNHIDE_KVER(6, 1, 0) ?
				 (void *)vpnhide_filldir_bool :
				 (void *)vpnhide_filldir_int;
	wrapper->pos = *(long long *)((char *)context + sizeof(void *));
	wrapper->original_context = context;
	wrapper->original_actor = *(void **)context;
	fargs->local.data4 = VPNHIDE_READDIR_MARKER;
	fargs->arg1 = (uint64_t)wrapper;
}

static void iterate_dir_after(hook_fargs2_t *fargs, void *udata)
{
	struct vpnhide_readdir_context *wrapper =
		(struct vpnhide_readdir_context *)&fargs->local;

	if (fargs->local.data4 != VPNHIDE_READDIR_MARKER ||
	    !wrapper->original_context)
		return;
	*(long long *)((char *)wrapper->original_context + sizeof(void *)) =
		wrapper->pos;
	fargs->arg1 = (uint64_t)wrapper->original_context;
}

/*
 * HOOK COVERAGE — parity with vpnhide_kmod.c (the .ko). CI exercises these
 * paths in booted QEMU images for 4.9, 4.14, 4.19, 5.4, 5.10, 5.15, 6.1,
 * 6.6, and 6.12. Filtering logic shared with the .ko lives in
 * shared/vpnhide_logic.h; version-specific field offsets live in
 * kver_offsets.h. QEMU coverage is the pre-merge safety gate, not a claim of
 * testing every vendor kernel or device configuration.
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
 *                                                   physical — QEMU matrix:
 *                                                   5.10/5.15/6.1/6.12 + legacy
 *                                                   5.4/4.19/4.14/4.9)
 *   rt6_fill_node          RTM_GETROUTE v6 dump   ✓ (fib6_info nexthop +
 *                                                   public /128 host-route via a
 *                                                   physical uplink, the .ko's
 *                                                   is_public_host_route6_via_
 *                                                   physical — QEMU matrix:
 *                                                   5.10/5.15/6.1/6.12 + legacy
 *                                                   5.4/4.19/4.14/4.9)
 *   fib_nl_fill_rule       RTM_GETRULE            ✓ (fib_rule iif/oif/uid)
 *   socket_bind_interface SO_BINDTODEVICE/index  ✓ (pre-mutation ENODEV;
 *                                                   state-aware raw-syscall test)
 *   filesystem_iface_paths sysfs/proc-sys VFS    ✓ (optional boot-time group;
 *                                                   lookup/open/stat/readdir)
 *   ( rt_fill_info — intentionally NOT hooked; unstable arg->reg ABI )
 *
 * Both host-route predicates (v4 + v6) and their address/iface logic are
 * shared with the .ko via shared/vpnhide_logic.h and used by every selected
 * offset table.
 * IPv4 needs no offset (fib_rt_info.dst/dst_len at a constant +12/+16 on 5.6+;
 * dst/dst_len passed as args 6/7 on <5.6). IPv6 reads the route's rt6key from
 * fib6_info.fib6_dst (64 on 5.4/4.19/5.10..6.6, 80 on 6.12) or, on the 4.14
 * 4.9/4.14 rt6_via_dst path, rt6_info.rt6i_dst (rt6_dst @256).
 */

/* ------------------------------------------------------------------ */
/*  Control plane                                                      */
/* ------------------------------------------------------------------ */

/*
 * The KPM has no file or node: its control/stats channel is the KernelPatch
 * `ctl0` supercall, carrying the §4 wire format (config in, stats/status out —
 * see vpnhide_kpm_ctl0). A procfs mirror is intentionally not created because
 * its proc_ops/file_operations ABI would add another kernel-version-dependent
 * structure layout. ctl0 avoids that dependency entirely.
 */

/* ------------------------------------------------------------------ */
/*  Init / exit                                                       */
/* ------------------------------------------------------------------ */

/*
 * Resolve a hook target by name, tolerating compiler-renamed clones. GCC may
 * emit a static function as `name.isra.N` / `name.constprop.N` (a specialised
 * clone) — kallsyms_lookup_name("name") then misses it. Clang with (Thin)LTO
 * promotes a local symbol to `name.llvm.<hash>` (seen on a vendor 5.4 build
 * where ipv6_route_seq_show became ipv6_route_seq_show.llvm.NNN while the plain
 * fib_route_seq_show stayed — leaving the IPv6 route hook uninstalled). The
 * clang-built GKI images usually retain the plain name, while gcc-built legacy
 * QEMU images can rename e.g. fib_nl_fill_rule -> fib_nl_fill_rule.isra.N.
 * Clang CFI with full LTO adds a fourth form, `name$<hex>` (vendor 4.14).
 * Fall back only to compiler clone forms observed in supported reference
 * kernels. Other dotted symbols (for example cold fragments, or the `.cfi_jt`
 * jump-table aliases) are not valid substitutes for the complete function.
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
	const char *suffix;
	int i;

	/* Hook targets are vmlinux functions; a same-named symbol exported by a
	 * loaded module is a different function entirely. */
	if (mod)
		return 0;
	for (i = 0; i < q->baselen; i++)
		if (name[i] != q->base[i])
			return 0;
	suffix = name + q->baselen;
	if (vpnhide_symbol_suffix_is_clone(suffix)) {
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

static int raw_usercopy_is_safe(void)
{
	uint64_t mmfr1;

	/* 4.9/4.14/4.19 __arch_copy_*_user assembly brackets the copy with
	 * uaccess_enable_not_uao itself. From 5.x onward that moved into the
	 * inline raw_copy_*_user wrapper, which may not have a callable symbol. */
	if ((unsigned int)kver < VPNHIDE_KVER(5, 0, 0))
		return 1;
	if (!_read_sanitised_ftr_reg)
		return 0;
	mmfr1 = _read_sanitised_ftr_reg(VPNHIDE_SYS_ID_AA64MMFR1_EL1);
	/* With hardware PAN, __arch_copy_*_user uses unprivileged LDTR/STTR
	 * accesses and does not need the software-PAN TTBR0 switch. */
	return ((mmfr1 >> VPNHIDE_ID_AA64MMFR1_PAN_SHIFT) & 0xfu) != 0;
}

static int resolve_symbols(void)
{
	/* Prefer the generic wrappers: they perform the software-PAN TTBR0 switch
	 * where required. Some builds inline every wrapper and expose only the raw
	 * architecture routines, so accept those only where raw_usercopy_is_safe()
	 * can prove they do not bypass software PAN. */
	_copy_from_user = (void *)kallsyms_lookup_name("_copy_from_user");
	_copy_to_user = (void *)kallsyms_lookup_name("_copy_to_user");
	if (!_copy_from_user || !_copy_to_user) {
		_read_sanitised_ftr_reg =
			(void *)kallsyms_lookup_name("read_sanitised_ftr_reg");
		if (!raw_usercopy_is_safe()) {
			logki(MODNAME
			      ": uaccess wrappers absent and raw copy is unsafe\n");
			return -1;
		}
		if (!_copy_from_user)
			_copy_from_user = (void *)kallsyms_lookup_name(
				"__arch_copy_from_user");
		if (!_copy_to_user)
			_copy_to_user = (void *)kallsyms_lookup_name(
				"__arch_copy_to_user");
		if (!_copy_from_user || !_copy_to_user) {
			logki(MODNAME ": user-copy symbols unavailable\n");
			return -1;
		}
		logki(MODNAME ": using safe raw user-copy routines\n");
	}

	/* Prefer skb_trim over __skb_trim. Both roll the skb back to the saved
	 * pre-fill length, but skb_trim guards with `if (skb->len > len)` using
	 * the kernel's own (correct) skb->len, so a wrong off->skb_len — which
	 * saves a bogus length on a vendor kernel whose sk_buff layout differs —
	 * degrades to a no-op instead of __skb_trim unconditionally pushing the
	 * tail past the allocation (out-of-bounds skb write -> slab corruption ->
	 * delayed panic). __skb_trim stays as a fallback only if skb_trim is
	 * somehow unresolvable. */
	_skb_trim = (void *)kallsyms_lookup_name("skb_trim");
	if (!_skb_trim)
		_skb_trim = (void *)kallsyms_lookup_name("__skb_trim");
	_netdev_get_name = (void *)lookup_fn("netdev_get_name");

	if (!_skb_trim) {
		logki(MODNAME ": skb trim helper unavailable\n");
		return -1;
	}
	return 0;
}

static int resolve_filesystem_symbols(void)
{
	_dentry_path_raw = (void *)kallsyms_lookup_name("dentry_path_raw");
	_vfs_statfs = (void *)kallsyms_lookup_name("vfs_statfs");
	_path_put = (void *)kallsyms_lookup_name("path_put");
	_fput = (void *)kallsyms_lookup_name("fput");
	if (_dentry_path_raw && _vfs_statfs && _path_put && _fput)
		return 0;
	logki(MODNAME
	      ": filesystem helpers unavailable (dentry_path_raw=%p vfs_statfs=%p path_put=%p fput=%p)\n",
	      _dentry_path_raw, _vfs_statfs, _path_put, _fput);
	return -1;
}

/* Parse and atomically apply the one supported config representation. Both the
 * runtime ctl0 path and the QEMU harness's optional load args use the frozen
 * control-v2 wire, so they cannot drift into separate parsers or semantics. */
static int apply_config(const char *wire, unsigned long wire_len)
{
	unsigned int default_mask = 0;
	int dbg = -1; /* absent debug record preserves live value */
	int i, n;

	if (!config_staging_try_claim())
		return -2;
	n = vpnhide_parse_config(wire, wire_len, config_staging,
				 MAX_TARGET_UIDS, &dbg, &default_mask);
	if (n < 0) {
		config_staging_release();
		return -1;
	}
	if (!cfg_try_write_begin()) {
		config_staging_release();
		return -2;
	}
	for (i = 0; i < n; i++) {
		__atomic_store_n(&target_uids[i], config_staging[i].uid,
				 __ATOMIC_RELAXED);
		__atomic_store_n(&target_masks[i],
				 config_staging[i].hookmask &
					 (VPNHIDE_KERNEL_HOOK_MASK |
					  VPNHIDE_KPM_HOOK_MASK),
				 __ATOMIC_RELAXED);
	}
	__atomic_store_n(&nr_targets, n, __ATOMIC_RELAXED);
	__atomic_store_n(&default_hookmask,
			 default_mask & (VPNHIDE_KERNEL_HOOK_MASK |
					 VPNHIDE_KPM_HOOK_MASK),
			 __ATOMIC_RELAXED);
	__atomic_store_n(&active_hook_mask, compute_active_hook_mask(n),
			 __ATOMIC_RELAXED);
	if (dbg >= 0)
		__atomic_store_n(&debug_enabled, dbg ? true : false,
				 __ATOMIC_RELAXED);
	cfg_write_end();
	config_staging_release();
	return 0;
}

/* Resolve `name`, wrap it, and record the install in `installed_hooks` so the
 * status channel (§4.3 `hooks`) reflects what actually took. */
static int install_hook(const char *name, int argno, void *before, void *after,
			enum vpnhide_hook_id hook_id)
{
	unsigned long fn = lookup_fn(name);
	struct vpnhide_hook_registration *registration;

	if (!fn || nr_hook_registrations >= VPNHIDE_MAX_HOOK_REGISTRATIONS)
		return 0;
	if (hook_wrap((void *)fn, argno, before, after, 0) == HOOK_NO_ERR) {
		registration = &hook_registrations[nr_hook_registrations++];
		registration->function = (void *)fn;
		registration->before = before;
		registration->after = after;
		installed_hooks |= vpnhide_hook_bit(hook_id);
		return 1;
	}
	return 0;
}

static void rollback_hook_registrations(unsigned int keep)
{
	while (nr_hook_registrations > keep) {
		struct vpnhide_hook_registration *registration =
			&hook_registrations[--nr_hook_registrations];

		hook_unwrap(registration->function, registration->before,
			    registration->after);
	}
}

static int install_filesystem_hooks(void)
{
	unsigned int registration_start = nr_hook_registrations;
	int getattr_argno = (unsigned int)kver < VPNHIDE_KVER(4, 11, 0) ? 2 : 4;

	if (resolve_filesystem_symbols() != 0)
		return 0;
	if (!install_hook("filename_lookup", 5, 0,
			  (void *)filename_lookup_after,
			  VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS) ||
	    !install_hook("do_filp_open", 3, 0, (void *)do_filp_open_after,
			  VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS) ||
	    !install_hook("vfs_getattr", getattr_argno, 0,
			  (void *)vfs_getattr_after,
			  VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS) ||
	    !install_hook("iterate_dir", 2, (void *)iterate_dir_before,
			  (void *)iterate_dir_after,
			  VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS)) {
		rollback_hook_registrations(registration_start);
		installed_hooks &=
			~vpnhide_hook_bit(VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS);
		return 0;
	}
	return 1;
}

static long vpnhide_kpm_init(const char *args, const char *event,
			     void *__user reserved)
{
	unsigned long args_len = 0;

	logki(MODNAME ": KPM init (event=%s) kver=0x%x\n", event ? event : "",
	      (unsigned int)kver);

	installed_hooks = 0;
	nr_hook_registrations = 0;
	last_error = VPNHIDE_ERR_OK;
	__atomic_store_n(&cfg_seq, 0, __ATOMIC_RELAXED);
	__atomic_store_n(&cfg_writer, 0, __ATOMIC_RELAXED);
	__atomic_store_n(&config_staging_writer, 0, __ATOMIC_RELAXED);
	__atomic_store_n(&nr_targets, 0,
			 __ATOMIC_RELAXED); /* empty until control v2 */
	__atomic_store_n(&default_hookmask, 0, __ATOMIC_RELAXED);
	__atomic_store_n(&active_hook_mask, 0, __ATOMIC_RELAXED);
	__atomic_store_n(&debug_enabled, false, __ATOMIC_RELAXED);
	filesystem_hiding_requested = false;
	_dentry_path_raw = 0;
	_vfs_statfs = 0;
	_path_put = 0;
	_fput = 0;

	/* `kver` is KernelPatch's running-kernel version (common.h), encoded
	 * the same way as VPNHIDE_KVER. NULL table = unsupported → bail. */
	off = vpnhide_select_offsets((unsigned int)kver);
	vfs_off = vpnhide_select_vfs_offsets((unsigned int)kver);
	if (!off || !vfs_off) {
		logki(MODNAME
		      ": unsupported kernel version — refusing to install\n");
		return -1; /* never guess offsets */
	}
	if (resolve_symbols() != 0) {
		logki(MODNAME ": symbol resolution failed\n");
		return -1;
	}

	/* Production passes the same boot-only switch as the .ko module parameter.
	 * The headless QEMU harness may instead supply its initial control-v2
	 * snapshot through load args; presence of the KPM-owned optional bit in that
	 * snapshot requests the same hook group. Runtime target config still uses
	 * ctl0 and cannot add or remove globally installed hooks. */
	if (args && args[0]) {
		if (string_equals(args, "filesystem_hiding=1")) {
			filesystem_hiding_requested = true;
		} else if (!string_equals(args, "filesystem_hiding=0")) {
			while (args[args_len])
				args_len++;
			if (apply_config(args, args_len) != 0) {
				logki(MODNAME ": invalid KPM load arguments\n");
				return -1;
			}
			filesystem_hiding_requested =
				(__atomic_load_n(&active_hook_mask,
						 __ATOMIC_RELAXED) &
				 VPNHIDE_KPM_HOOK_MASK) != 0;
		}
	}

	/*
	 * Install hooks. Fields used as availability gates are nonzero only where
	 * the corresponding layout is defined. Zero is also a valid offset for
	 * several fields, so it is not a general validity marker; every selected
	 * table must still be derived and tested as a complete unit. seq_file and
	 * ioctl hooks use layouts shared by all supported tables. install_hook
	 * records each successful registration in installed_hooks.
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
	install_hook("sock_ioctl", 3, (void *)sock_ioctl_before,
		     (void *)sock_ioctl_after, VPNHIDE_HOOK_SOCK_IOCTL);
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
	if (_netdev_get_name && off->sock_net && off->socket_sk) {
		int bind_ok;

		if (socket_bind_uses_index_hook()) {
			/* 5.7-5.8 still pass a raw user pointer to sock_setsockopt.
			 * Hook the resolved index instead, after the user copy and name
			 * lookup but before sk_bound_dev_if changes. A missing static
			 * symbol leaves status partial rather than accepting a TOCTOU. */
			bind_ok = install_hook(
				socket_bind_index_hook_name(), 2,
				(void *)socket_bind_index_before, 0,
				VPNHIDE_HOOK_SOCKET_BIND_INTERFACE);
		} else {
			bind_ok = install_hook(
				"sock_setsockopt", 6,
				(void *)socket_bind_sock_before, 0,
				VPNHIDE_HOOK_SOCKET_BIND_INTERFACE);

			/* 6.1+ generic sockets may have sk_setsockopt inlined into
			 * the exported wrapper under LTO, while MPTCP calls the
			 * standalone symbol directly. Cover both call graphs. */
			if (sockopt_takes_sk())
				bind_ok =
					install_hook(
						"sk_setsockopt", 6,
						(void *)socket_bind_sk_before,
						0,
						VPNHIDE_HOOK_SOCKET_BIND_INTERFACE) &&
					bind_ok;
		}
		if (!bind_ok)
			installed_hooks &= ~vpnhide_hook_bit(
				VPNHIDE_HOOK_SOCKET_BIND_INTERFACE);
	}
	if (filesystem_hiding_requested && !install_filesystem_hooks())
		logki(MODNAME
		      ": optional filesystem hook group failed atomically\n");

	/* Healthy iff every requested hook installed; otherwise honestly
	 * report partial — the `hooks` mask carries which ones (§5.1). A kver
	 * with an incomplete offset table lands here by design. */
	last_error =
		(installed_hooks ==
		 (VPNHIDE_KERNEL_HOOK_MASK |
		  (filesystem_hiding_requested ? VPNHIDE_KPM_HOOK_MASK : 0))) ?
			VPNHIDE_ERR_OK :
			VPNHIDE_ERR_PARTIAL_HOOKS;

	logki(MODNAME
	      ": KPM hooks installed (mask=0x%x filesystem_hiding=%d err=%u)\n",
	      installed_hooks, filesystem_hiding_requested ? 1 : 0, last_error);
	return 0;
}

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
		/* A bad header/version never touches live state; a concurrent ctl0
		 * writer gets -2 (busy) and userspace retries. */
		long result = apply_config(args, n_args);

		if (result == 0)
			vpnhide_dbg("ctl0 config applied, debug=%d\n",
				    __atomic_load_n(&debug_enabled,
						    __ATOMIC_RELAXED) ?
					    1 :
					    0);
		return result;
	}

	if (kind == VPNHIDE_KIND_STATS || kind == VPNHIDE_KIND_STATUS) {
		char buf[VPNHIDE_OUT_MAX];
		unsigned long available, full, n;

		if (kind == VPNHIDE_KIND_STATS) {
			unsigned int after;

			if (!vpnhide_parse_stats_after(args, n_args, &after))
				return -1;
			available = outlen > 0 ? (unsigned long)outlen : 0;
			if (available > sizeof(buf))
				available = sizeof(buf);
			n = format_stats_page(buf, available, after);
			return copy_ctl_reply(out_msg, buf, n);
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
		 * exceed sizeof(buf); preserve that larger value so
		 * clamp_to_line knows the snapshot is incomplete even when the
		 * caller also supplied a 4096-byte buffer. Limit only the bytes it
		 * may inspect/copy to the stack buffer actually written. This
		 * guarantees a truncated reply ends at a complete line with its
		 * final newline removed (the protocol truncation signal).
		 */
		available = outlen > 0 ? (unsigned long)outlen : 0;
		if (available > sizeof(buf))
			available = sizeof(buf);
		n = vpnhide_clamp_to_line(buf, full, available);
		return copy_ctl_reply(out_msg, buf, n);
	}

	return -1; /* unknown kind */
}

static long vpnhide_kpm_exit(void *__user reserved)
{
	rollback_hook_registrations(0);
	installed_hooks = 0;

	logki(MODNAME ": KPM unloaded\n");
	return 0;
}

KPM_INIT(vpnhide_kpm_init);
KPM_CTL0(vpnhide_kpm_ctl0);
KPM_EXIT(vpnhide_kpm_exit);
