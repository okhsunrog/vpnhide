/* SPDX-License-Identifier: MIT */
/*
 * Per-kernel-version struct-offset table for the KPM backend.
 *
 * WHY THIS EXISTS
 * ---------------
 * KernelPatch resolves *symbol addresses* at load time (runtime
 * `kallsyms_lookup_name`), so one `.kpm` binary loads across many kernels.
 * It does NOT give us struct *field* offsets — those changed across
 * versions and the compiler can't compute them because we never include
 * the real kernel headers (`-nostdinc`). So we keep a small offset table
 * keyed on the running kernel version, selected at init.
 *
 * This is the difference between our approach and soranerai's KPM, which
 * ships three near-identical source files (4.14 / 5.4 / 6.1) with the
 * offsets baked in. One source + one runtime table = one binary for all.
 *
 * VALUES MUST BE PROVEN, NOT GUESSED. Each non-TODO value below is either
 * (a) confirmed by soranerai on a real device, or (b) trivially stable
 * (e.g. net_device.name is the first member on every version). Every new
 * value has to pass the QEMU KPM harness (see kmod/kpm/README.md) before
 * it ships — a wrong offset here is a kernel panic / bootloop, not a soft
 * failure like a kretprobe that won't register.
 */
#ifndef VPNHIDE_KVER_OFFSETS_H
#define VPNHIDE_KVER_OFFSETS_H

/* KernelPatch exposes the running kernel version as `kver` (see
 * <kpmodule.h>). Encoding matches LINUX_VERSION_CODE: (a<<16)|(b<<8)|c. */
#define VPNHIDE_KVER(a, b, c) (((a) << 16) + ((b) << 8) + (c))

struct vpnhide_offsets {
	/* sk_buff.len — saved before a netlink fill, restored via skb_trim. */
	unsigned int skb_len;

	/* net_device.name — first member (@0) up to 6.6; 6.12 moved it behind
	 * the new cacheline-group reorg (@296). */
	unsigned int netdev_name;

	/* seq_file.{buf,count} — stable across the seq_file lifetime. */
	unsigned int seqfile_buf;
	unsigned int seqfile_count;

	/* in_ifaddr.ifa_dev (-> in_device.dev -> net_device).  [TODO 5.4/4.14] */
	unsigned int in_ifaddr_ifa_dev;
	unsigned int in_device_dev;

	/* inet6_ifaddr.idev (-> inet6_dev.dev -> net_device). [TODO 5.4/4.14] */
	unsigned int inet6_ifaddr_idev;
	unsigned int inet6_dev_dev;

	/* inet{,6}_fill_ifaddr ABI:
	 *   4.x:  (skb, ifa, portid, seq, event, flags) => hook_wrap argno 6
	 *   5.x+: (skb, ifa, fillargs)                  => hook_wrap argno 3 */
	unsigned int addr_fill_argno;

	/* IPv4 route dump (fib_dump_info): fib_rt_info.fi=0, fib_nh_common.nhc_dev=0
	 * (constants). These three vary per version; for the legacy single-nexthop
	 * route: dev = *(fi + fib_info_fib_nh + nhc_dev). fib_info_nh != 0 marks a
	 * nexthop-object route (not unpacked yet). 0 => hook bails (no guessing). */
	unsigned int fib_info_fib_nhs;
	unsigned int fib_info_nh;
	unsigned int fib_info_fib_nh;

	/* fib_dump_info arg shape (its fib_info moved across versions):
	 *   fib_dump_fi_arg     = args[] index of the fib_info/fib_rt_info ptr
	 *                         (0 => hook not installed for this version)
	 *   fib_dump_fi_via_fri = 1 if that arg is a fib_rt_info* (fi=*(arg+0),
	 *                         5.6+), 0 if it is the fib_info* directly (<5.6).
	 * Always hooked as a 12-arg frame; only this one arg is read. */
	unsigned int fib_dump_fi_arg;
	int fib_dump_fi_via_fri;

	/* IPv6 route dump (rt6_fill_node): fib6_info.nh != 0 => nexthop object;
	 * else dev = *(rt + fib6_info_fib6_nh) (fib6_nh[0].nhc_dev). */
	unsigned int fib6_info_nh;
	unsigned int fib6_info_fib6_nh;

	/* fib6_info.fib6_dst (struct rt6key { in6_addr addr@0; int plen@16; }) —
	 * the destination of an IPv6 route, used to spot a public /128 host-route
	 * pinned to a physical uplink. 0 disables that check for this version
	 * (e.g. the pre-fib6_info 4.14 rt6_info path, and the non-GKI kernels that
	 * the QEMU matrix can't validate — a wrong offset here would fault). */
	unsigned int fib6_info_fib6_dst;

	/* Pre-fib6_info kernels (<= 4.14): rt6_fill_node takes a struct rt6_info*
	 * (which begins with a struct dst_entry), not a fib6_info. When
	 * rt6_via_dst is set, dev = *(rt + rt6_dst_dev) (dst_entry.dev), and the
	 * route's destination is rt6_info.rt6i_dst (a rt6key) at rt6_dst (the
	 * fib6_info_fib6_dst above is for the fib6_info path; 0 disables the v6
	 * host-route check). */
	int rt6_via_dst;
	unsigned int rt6_dst_dev;
	unsigned int rt6_dst;

	/* Policy rules (fib_nl_fill_rule -> struct fib_rule). */
	unsigned int fib_rule_table;
	unsigned int fib_rule_iifname;
	unsigned int fib_rule_oifname;
	unsigned int fib_rule_uid_start;
	unsigned int fib_rule_uid_end;

	/* 1 if procfs uses `struct proc_ops` (>=5.6), 0 if `file_operations`
	 * (<5.6). Mixing these up is the most likely cause of the
	 * /proc/vpnhide_targets crash reported on HyperOS 5.4. */
	int proc_uses_proc_ops;
};

/*
 * GKI 6.1 (android14-6.1, derived from AOSP common source + QEMU-validated).
 * Also covers 6.6 (android15-6.6): every offset here is byte-identical on 6.6,
 * confirmed by a separate 9/9 harness run on a DDK-built 6.6 GKI Image.
 * inet6_ifaddr's prefix up to dad_work is identical to 5.10, so idev@168
 * (NOT the 216 first taken from soranerai — left a note, the harness decides).
 * fib_dump_info uses the fib_rt_info* form (arg 4, like 5.10); fib_info and
 * fib_rule layouts match 5.10; fib6_info has an ANDROID_KABI_RESERVE before
 * fib6_nh[]. procfs is proc_ops (>=5.6).
 */
static const struct vpnhide_offsets vpnhide_off_6_1 = {
	.skb_len = 112, /* modern head + _nfct (conntrack on in GKI 6.1) */
	.netdev_name = 0,
	.seqfile_buf = 0,
	.seqfile_count = 24,
	.in_ifaddr_ifa_dev = 24,
	.in_device_dev = 0,
	.inet6_ifaddr_idev = 168, /* same struct prefix as validated 5.10 */
	.inet6_dev_dev = 0,
	.addr_fill_argno = 3,
	/* fib_info identical to 5.10: fib_nhs@96, nh@104, fib_nh[]@128. */
	.fib_info_fib_nhs = 96,
	.fib_info_nh = 104,
	.fib_info_fib_nh = 128,
	.fib_dump_fi_arg = 4, /* fib_rt_info* form (5.6+) */
	.fib_dump_fi_via_fri = 1,
	/* fib6_info: bitfield@136, rcu@144, nh@160, ANDROID_KABI_RESERVE(1)@168,
	 * fib6_nh[]@176; nhc_dev first in fib6_nh -> dev@176. fib6_dst is the first
	 * rt6key after fib6_metrics@56 -> @64 (no gc_link, same head as 5.10). */
	.fib6_info_nh = 160,
	.fib6_info_fib6_nh = 176,
	.fib6_info_fib6_dst = 64,
	/* struct fib_rule layout identical to 5.10. */
	.fib_rule_table = 36,
	.fib_rule_iifname = 88,
	.fib_rule_oifname = 104,
	.fib_rule_uid_start = 120,
	.fib_rule_uid_end = 124,
	.proc_uses_proc_ops = 1,
};

/* 6.12 (android16-6.12) — QEMU-validated 9/9. Identical to 6.1 EXCEPT struct
 * fib6_info inserted `struct hlist_node gc_link` (16 B) after `expires`,
 * shifting nh@176 and fib6_nh[]@192 (6.1 is 160/176). (fib_info also gained a
 * `pfsrc_removed` bool, but it fits the existing pad so nh stays @104.) */
static const struct vpnhide_offsets vpnhide_off_6_12 = {
	.skb_len = 112,
	/* 6.12 reorganised struct net_device into cacheline groups (TX/TXRX/RX
	 * read-mostly), so `name` is no longer first — offsetof = 296 (derived by
	 * compiling offsetof(struct net_device, name) against the gki_defconfig
	 * 6.12 headers). Every other version keeps name@0. */
	.netdev_name = 296,
	.seqfile_buf = 0,
	.seqfile_count = 24,
	.in_ifaddr_ifa_dev = 24,
	.in_device_dev = 0,
	.inet6_ifaddr_idev = 168,
	.inet6_dev_dev = 0,
	.addr_fill_argno = 3,
	.fib_info_fib_nhs = 96,
	.fib_info_nh = 104,
	.fib_info_fib_nh = 128,
	.fib_dump_fi_arg = 4,
	.fib_dump_fi_via_fri = 1,
	/* fib6_info + gc_link(16) before fib6_metrics -> nh@176, fib6_nh[]@192.
	 * The same gc_link shifts fib6_metrics@72 and fib6_dst@80 (vs @64 elsewhere). */
	.fib6_info_nh = 176,
	.fib6_info_fib6_nh = 192,
	.fib6_info_fib6_dst = 80,
	.fib_rule_table = 36,
	.fib_rule_iifname = 88,
	.fib_rule_oifname = 104,
	.fib_rule_uid_start = 120,
	.fib_rule_uid_end = 124,
	.proc_uses_proc_ops = 1,
};

/* 5.10 (android12-5.10) — QEMU-validated 9/9. (5.15 has a different fib6_info
 * and gets its own table below.) */
static const struct vpnhide_offsets vpnhide_off_5_x = {
	.skb_len = 112, /* GKI 5.10 has CONFIG_NF_CONNTRACK=y: _nfct@104,
			 * len@112. Reading 104 trims netlink dumps to garbage and
			 * drops non-VPN rows alongside vpn0. */
	.netdev_name = 0,
	.seqfile_buf = 0,
	.seqfile_count = 24,
	/* in_ifaddr: hash(16)+ifa_next(8) -> ifa_dev@24; in_device.dev@0. */
	.in_ifaddr_ifa_dev = 24,
	.in_device_dev = 0,
	/* inet6_ifaddr.idev@168 (derived; past lock/dad_work — verify via gai),
	 * inet6_dev.dev@0. */
	.inet6_ifaddr_idev = 168,
	.inet6_dev_dev = 0,
	.addr_fill_argno = 3,
	/* android12-5.10 fib_info (LP64, no ifdefs before fib_nh[]): fib_nhs@96,
	 * nh@104, fib_nh[]@128; nhc_dev is first in fib_nh -> +128. */
	.fib_info_fib_nhs = 96,
	.fib_info_nh = 104,
	.fib_info_fib_nh = 128,
	/* 5.6+ fib_dump_info(skb, portid, seq, event, fib_rt_info *fri, flags):
	 * fi = fri->fi, so arg index 4, dereffed through fri. */
	.fib_dump_fi_arg = 4,
	.fib_dump_fi_via_fri = 1,
	/* android12-5.10 fib6_info (LP64; rt6key=20, ANDROID_KABI_RESERVE=8):
	 * nh@152, fib6_nh[]@168; nhc_dev first in fib6_nh -> +168. fib6_dst is the
	 * first rt6key after fib6_metrics@56 -> @64. */
	.fib6_info_nh = 152,
	.fib6_info_fib6_nh = 168,
	.fib6_info_fib6_dst = 64,
	/* android12-5.10 struct fib_rule (LP64). */
	.fib_rule_table = 36,
	.fib_rule_iifname = 88,
	.fib_rule_oifname = 104,
	.fib_rule_uid_start = 120,
	.fib_rule_uid_end = 124,
	.proc_uses_proc_ops = 1, /* 5.6+; for <5.6 use file_operations below */
};

/* 5.15 (android13-5.15, derived from source + QEMU-validated). Identical to
 * 5.10 EXCEPT struct fib6_info gained offload/offload_failed fields (same
 * shape as 6.1), pushing nh@160 and fib6_nh[]@176 (5.10 is 152/168). This is
 * why 5.15 can't share the 5.10 table — its IPv6 route-dump dev would be
 * misread. Everything else (idev@168, fib_info 96/104/128, fib_dump via
 * fib_rt_info*@arg4, fib_rule) matches 5.10. */
static const struct vpnhide_offsets vpnhide_off_5_15 = {
	.skb_len = 112, /* GKI 5.15 has CONFIG_NF_CONNTRACK=y: _nfct@104,
			 * len@112. Same rollback requirement as 5.10. */
	.netdev_name = 0,
	.seqfile_buf = 0,
	.seqfile_count = 24,
	.in_ifaddr_ifa_dev = 24,
	.in_device_dev = 0,
	.inet6_ifaddr_idev = 168,
	.inet6_dev_dev = 0,
	.addr_fill_argno = 3,
	.fib_info_fib_nhs = 96,
	.fib_info_nh = 104,
	.fib_info_fib_nh = 128,
	.fib_dump_fi_arg = 4,
	.fib_dump_fi_via_fri = 1,
	/* fib6_info with offload/offload_failed (cf. 6.1): nh@160, fib6_nh[]@176.
	 * The offload fields are after the rt6keys, so fib6_dst stays @64. */
	.fib6_info_nh = 160,
	.fib6_info_fib6_nh = 176,
	.fib6_info_fib6_dst = 64,
	.fib_rule_table = 36,
	.fib_rule_iifname = 88,
	.fib_rule_oifname = 104,
	.fib_rule_uid_start = 120,
	.fib_rule_uid_end = 124,
	.proc_uses_proc_ops = 1,
};

/* 5.4 (android11-5.4, derived from AOSP common source + QEMU-validated).
 * Mostly identical to 5.10, with two version-specific differences:
 *   - fib_dump_info still uses the legacy <5.6 prototype (fib_info* passed
 *     directly at arg 9, 11 args) — not the fib_rt_info* form 5.10 has.
 *   - struct fib6_info has no ANDROID_KABI_RESERVE before fib6_nh[] here, so
 *     fib6_nh[]@160 (vs 5.10's 168).
 * procfs is file_operations (<5.6). */
static const struct vpnhide_offsets vpnhide_off_5_4 = {
	.skb_len = 112, /* modern sk_buff head + _nfct (CONFIG_NF_CONNTRACK) ->
			 * len@112. Real 5.4 devices carry conntrack, so this is
			 * the device-faithful value; a conntrack-less kernel would
			 * be @104 (cf. the 5.10 test kernel). */
	.netdev_name = 0,
	.seqfile_buf = 0,
	.seqfile_count = 24,
	.in_ifaddr_ifa_dev = 24, /* hash(16)+ifa_next(8) — same as 5.10 */
	.in_device_dev = 0,
	.inet6_ifaddr_idev =
		168, /* rt_priority present + post-4.15 timer_list */
	.inet6_dev_dev = 0,
	.addr_fill_argno = 3,
	/* fib_info identical to 5.10: fib_nhs@96, nh@104, fib_nh[]@128. */
	.fib_info_fib_nhs = 96,
	.fib_info_nh = 104,
	.fib_info_fib_nh = 128,
	/* legacy fib_dump_info(skb,...,tos, struct fib_info *fi, flags): fi@arg9,
	 * passed directly (not via fib_rt_info). */
	.fib_dump_fi_arg = 9,
	.fib_dump_fi_via_fri = 0,
	/* fib6_info: rt6key=20, NO KABI reserve -> nh@152, fib6_nh[]@160. fib6_dst
	 * is the first rt6key after fib6_metrics@56 -> @64 (head identical to 5.10);
	 * QEMU-validated on the legacy 5.4 image (kpm-qemu-legacy). */
	.fib6_info_nh = 152,
	.fib6_info_fib6_nh = 160,
	.fib6_info_fib6_dst = 64,
	/* struct fib_rule layout identical to 5.10. */
	.fib_rule_table = 36,
	.fib_rule_iifname = 88,
	.fib_rule_oifname = 104,
	.fib_rule_uid_start = 120,
	.fib_rule_uid_end = 124,
	.proc_uses_proc_ops = 0, /* <5.6 → file_operations */
};

/* 4.19 (vanilla 4.19.325, derived from source + QEMU-validated). Pre-nexthop
 * kernel: fib_info/fib6_info have NO `struct nexthop *nh` field (so the nh
 * guards in dev_from_fib*_info skip), and fib_dump_info uses the legacy <5.6
 * prototype (fib_info* directly at arg 9). Unlike 4.14 it already has the
 * struct fib6_info IPv6 route model (4.14 still uses rt6_info). procfs is
 * file_operations (<5.6). */
static const struct vpnhide_offsets vpnhide_off_4_19 = {
	/* XFRM + conntrack + bridge-netfilter fields put sk_buff.len at 128 on
	 * the 4.19 image we validate with. A smaller value trims netlink dumps
	 * back to a pointer field and can make getifaddrs() look completely empty. */
	.skb_len = 128,
	.netdev_name = 0,
	.seqfile_buf = 0,
	.seqfile_count = 24,
	.in_ifaddr_ifa_dev = 24,
	.in_device_dev = 0,
	.inet6_ifaddr_idev =
		168, /* rt_priority present, post-4.15 timer_list */
	.inet6_dev_dev = 0,
	.addr_fill_argno = 6,
	/* fib_info: fib_nhs@80, rcu@88, fib_nh[]@104; fib_nh.nh_dev is first ->
	 * dev = *(fi+104). No nexthop objects (fib_info_nh = 0). */
	.fib_info_fib_nhs = 80,
	.fib_info_nh = 0,
	.fib_info_fib_nh = 104,
	.fib_dump_fi_arg = 9, /* legacy prototype: fib_info* at arg 9 */
	.fib_dump_fi_via_fri = 0,
	/* fib6_info (no nexthop objects): fib6_nh embedded (not fib_nh_common),
	 * nh_dev is the 2nd field (@+16 after in6_addr nh_gw). With
	 * CONFIG_IPV6_ROUTER_PREF (Android-common) fib6_nh@160 -> nh_dev@176.
	 * (Without ROUTER_PREF it would be @168 — config-sensitive.) */
	.fib6_info_nh = 0,
	.fib6_info_fib6_nh = 176,
	/* 4.19 fib6_info: list_head fib6_siblings (16, no union) then nsiblings/
	 * ref/expires/metrics -> fib6_dst, the first rt6key, lands @64 (same as 5.4/
	 * 5.10). QEMU-validated on the legacy 4.19 image. */
	.fib6_info_fib6_dst = 64,
	/* struct fib_rule layout identical to 5.4/5.10. */
	.fib_rule_table = 36,
	.fib_rule_iifname = 88,
	.fib_rule_oifname = 104,
	.fib_rule_uid_start = 120,
	.fib_rule_uid_end = 124,
	.proc_uses_proc_ops = 0, /* <5.6 → file_operations */
};

/* 4.14 (vanilla 4.14.336, derived from source + QEMU-validated, 9/9). Oldest
 * target and the most structurally different: fib_dump_info uses the legacy
 * <5.6 prototype (fi at arg 9), there are no nexthop objects, and IPv6 still
 * uses struct rt6_info (not fib6_info) — so the IPv6 dev comes from the
 * embedded dst_entry (rt6_via_dst) rather than a fib6_nh walk. procfs is
 * file_operations (<5.6). */
static const struct vpnhide_offsets vpnhide_off_4_x = {
	/* XFRM + conntrack + bridge-netfilter fields put sk_buff.len at 128 on
	 * the 4.14 images we validate with, including sunfish 4.14.302. */
	.skb_len = 128,
	.netdev_name = 0,
	.seqfile_buf = 0,
	.seqfile_count = 24,
	.in_ifaddr_ifa_dev =
		24, /* in_ifaddr: hash(16)+ifa_next(8) — same as 5.x */
	.in_device_dev = 0,
	.inet6_ifaddr_idev =
		168, /* no rt_priority (-4) but timer_list has `data` (+8) -> 168 */
	.inet6_dev_dev = 0,
	.addr_fill_argno = 6,
	/* fib_info: fib_nhs@80, rcu@88, fib_nh[]@104 (fib_weight, if
	 * CONFIG_IP_ROUTE_MULTIPATH, fills the pad slot @84 so fib_nh stays @104);
	 * fib_nh.nh_dev is first. No nexthop objects. */
	.fib_info_fib_nhs = 80,
	.fib_info_nh = 0,
	.fib_info_fib_nh = 104,
	.fib_dump_fi_arg = 9, /* legacy prototype: fib_info* at arg 9 */
	.fib_dump_fi_via_fri = 0,
	/* IPv6: rt6_fill_node(rt6_info*). rt6_info begins with struct dst_entry,
	 * whose first member is net_device *dev -> dev = *(rt + 0). */
	.fib6_info_nh = 0,
	.fib6_info_fib6_nh = 0,
	.rt6_via_dst = 1,
	.rt6_dst_dev = 0,
	/* rt6_info.rt6i_dst (rt6key) is ____cacheline_aligned_in_smp: dst_entry is
	 * 160B (config-independent — the __pads keep it constant), the rt6_info
	 * prefix runs to +220, rounded up to a cacheline -> @256 (same for 64- or
	 * 128-byte lines). QEMU-validated on the legacy 4.14 image. */
	.rt6_dst = 256,
	/* struct fib_rule layout matches 5.10; the symbol is fib_nl_fill_rule
	 * .isra.N under gcc — the fuzzy resolver finds it. */
	.fib_rule_table = 36,
	.fib_rule_iifname = 88,
	.fib_rule_oifname = 104,
	.fib_rule_uid_start = 120,
	.fib_rule_uid_end = 124,
	.proc_uses_proc_ops = 0, /* <5.6 → file_operations */
};

/*
 * Select the offset table for the running kernel. Returns NULL for an
 * unsupported version — the caller MUST refuse to install hooks then
 * (kpm-spore does the same: unknown version → bail, never guess).
 */
static inline const struct vpnhide_offsets *
vpnhide_select_offsets(unsigned int kver)
{
	if (kver >= VPNHIDE_KVER(6, 12, 0))
		return &vpnhide_off_6_12; /* 6.12: fib6_info gained gc_link */
	if (kver >= VPNHIDE_KVER(6, 0, 0))
		return &vpnhide_off_6_1; /* 6.0–6.11 — 6.1 + 6.6 proven identical */
	if (kver >= VPNHIDE_KVER(5, 15, 0))
		return &vpnhide_off_5_15; /* 5.15+: fib6_info gained offload fields */
	if (kver >= VPNHIDE_KVER(5, 6, 0))
		return &vpnhide_off_5_x; /* 5.6–5.14 */
	if (kver >= VPNHIDE_KVER(5, 0, 0))
		return &vpnhide_off_5_4; /* 5.0–5.5: legacy fib_dump_info + proc */
	if (kver >= VPNHIDE_KVER(4, 19, 0))
		return &vpnhide_off_4_19; /* 4.19: fib6_info, no nexthop objects */
	if (kver >= VPNHIDE_KVER(4, 0, 0))
		return &vpnhide_off_4_x;
	return 0; /* unsupported → do not install */
}

#endif /* VPNHIDE_KVER_OFFSETS_H */
