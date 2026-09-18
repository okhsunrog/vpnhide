// SPDX-License-Identifier: MIT
/*
 * ABI probe: a translation unit whose only purpose is to make the compiler
 * emit full DWARF for every kernel structure the backends dereference, so
 * `pahole` can print their sizes and member offsets.
 *
 * WHY THIS EXISTS
 * ---------------
 * `kmod/test/qemu-trim.config` switches off subsystems the QEMU harness never
 * exercises. A Kconfig symbol can change a *struct layout* (a member behind
 * `#ifdef`, a subsystem whose objects embed a pointer in `struct sock` or
 * `struct net_device`), and the KPM backend reads those layouts from the
 * hardcoded table in `kmod/kpm/kver_offsets.h` — it has no headers to recompile
 * against. A trim that shifted one offset would make `kpm-qemu` either fail, or
 * worse, pass against a layout no real device has.
 *
 * So `kmod/test/verify-trim-abi.sh` builds this file twice against the same
 * kernel tree — once with the trim, once without — and diffs the pahole output.
 * Any difference means the fragment disabled something structural.
 *
 * Nothing here runs. The object file is compiled and thrown away; only its
 * debug info is read. Instantiating each type as a variable is what forces the
 * compiler to emit its complete definition rather than a forward declaration.
 *
 * The list below is the union of the structures dereferenced by the three
 * native backends (`kmod/vpnhide_kmod.c`, `kmod/kpm/vpnhide_kpm.c` and the
 * hook sources under `builtin/security/vpnhide/`) plus the core canaries the
 * brief calls for (`task_struct`, `device`) — a size change there shifts the
 * tail of anything that embeds them. Structures defined inside a kernel .c
 * file (the inet fill-args argument packs, `struct open_flags`) cannot be
 * included from here and are out of scope.
 */

#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/version.h>
#include <linux/kprobes.h>
#include <linux/cred.h>
#include <linux/sched.h>
#include <linux/device.h>
#include <linux/net.h>
#include <linux/if.h>
#include <linux/seq_file.h>
#include <linux/proc_fs.h>
#include <linux/fs.h>
#include <linux/stat.h>
#include <linux/statfs.h>
#include <linux/namei.h>
#include <linux/file.h>
#include <linux/nsproxy.h>
#include <linux/netdevice.h>
#include <linux/rtnetlink.h>
#include <linux/skbuff.h>
#include <linux/inetdevice.h>
#include <net/sock.h>
#include <net/if_inet6.h>
#include <net/ip_fib.h>
#include <net/nexthop.h>
#include <net/ip6_fib.h>
#include <net/ip6_route.h>
#include <net/route.h>
#include <net/fib_rules.h>

/*
 * One definition per type. `volatile` keeps a zealous compiler from discarding
 * the (never read) object along with its debug info.
 */
#define VPNHIDE_ABI_PROBE(type) volatile struct type vpnhide_abi_##type

/* Socket / networking core — the layouts every backend walks. */
VPNHIDE_ABI_PROBE(sock);
VPNHIDE_ABI_PROBE(sock_common);
VPNHIDE_ABI_PROBE(socket);
VPNHIDE_ABI_PROBE(sk_buff);
VPNHIDE_ABI_PROBE(net_device);
VPNHIDE_ABI_PROBE(net);
VPNHIDE_ABI_PROBE(nsproxy);
VPNHIDE_ABI_PROBE(dst_entry);

/* Address lists (getifaddrs / rtnetlink address dumps). */
VPNHIDE_ABI_PROBE(in_ifaddr);
VPNHIDE_ABI_PROBE(in_device);
VPNHIDE_ABI_PROBE(inet6_ifaddr);
VPNHIDE_ABI_PROBE(inet6_dev);

/* Route and policy-rule dumps. */
VPNHIDE_ABI_PROBE(fib_info);
VPNHIDE_ABI_PROBE(fib_nh);
VPNHIDE_ABI_PROBE(fib_nh_common);
VPNHIDE_ABI_PROBE(fib_rt_info);
VPNHIDE_ABI_PROBE(fib_rule);
VPNHIDE_ABI_PROBE(nexthop);
VPNHIDE_ABI_PROBE(nh_info);
VPNHIDE_ABI_PROBE(nh_group);
VPNHIDE_ABI_PROBE(rtable);
VPNHIDE_ABI_PROBE(fib6_info);
VPNHIDE_ABI_PROBE(fib6_nh);
VPNHIDE_ABI_PROBE(rt6_info);
VPNHIDE_ABI_PROBE(rt6key);

/* ioctl argument packs (SIOCGIFCONF / SIOCGIFINDEX and friends). */
VPNHIDE_ABI_PROBE(ifreq);
VPNHIDE_ABI_PROBE(ifconf);

/* /proc rendering and the VFS objects the filesystem-hiding hooks touch. */
VPNHIDE_ABI_PROBE(seq_file);
VPNHIDE_ABI_PROBE(seq_operations);
VPNHIDE_ABI_PROBE(proc_ops);
VPNHIDE_ABI_PROBE(file);
VPNHIDE_ABI_PROBE(file_operations);
VPNHIDE_ABI_PROBE(file_system_type);
VPNHIDE_ABI_PROBE(path);
VPNHIDE_ABI_PROBE(dentry);
VPNHIDE_ABI_PROBE(qstr);
VPNHIDE_ABI_PROBE(inode);
VPNHIDE_ABI_PROBE(filename);
VPNHIDE_ABI_PROBE(dir_context);
VPNHIDE_ABI_PROBE(kstat);
VPNHIDE_ABI_PROBE(kstatfs);

/* Probe plumbing (the .ko attaches through these). */
VPNHIDE_ABI_PROBE(pt_regs);
VPNHIDE_ABI_PROBE(kprobe);
VPNHIDE_ABI_PROBE(kretprobe);
VPNHIDE_ABI_PROBE(kretprobe_instance);

/* Core canaries: a size change here shifts the tail of everything embedding
 * them (`struct device` is embedded in `struct net_device`). */
VPNHIDE_ABI_PROBE(task_struct);
VPNHIDE_ABI_PROBE(cred);
VPNHIDE_ABI_PROBE(device);
VPNHIDE_ABI_PROBE(module);

MODULE_LICENSE("GPL");
MODULE_DESCRIPTION("vpnhide test-only ABI layout probe (never loaded)");
