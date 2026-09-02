#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate the vpnhide call-site patches for a KMI from a real kernel tree.

Hand-written unified diffs are fragile (a fuzzed hunk can land at the wrong
offset). Instead we apply anchor-based, uniqueness-checked string insertions to
a clean `kernel/common` git tree, `git diff` the result into per-file
`.patch` files under versions/<kmi>/, then restore the tree. apply.sh consumes
the emitted patches; this script only produces them.

    gen_patches.py <kernel_tree> [--kmi android14-6.1] [--keep]

The tree must be a clean git checkout of the matching KMI. Each edit asserts its
anchor occurs exactly once, so a kernel whose source drifted enough to move or
duplicate an anchor fails loudly here rather than silently mis-patching.
"""
from __future__ import annotations

import argparse
import copy
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

VH_INCLUDE = "#include <linux/vpnhide.h>\n"

# Per-KMI edits: {relpath: [(anchor, replacement), ...]}. Each anchor must be
# unique in the file; replacement contains the anchor so the edit is a pure
# insertion. Keep the inserted calls guard-free — <linux/vpnhide.h> stubs to
# `return false`/PASSTHROUGH when CONFIG_VPNHIDE=n, so the sites vanish.
EDITS: dict[str, dict[str, list[tuple[str, str]]]] = {
    "android14-6.1": {
        "net/core/dev_ioctl.c": [
            ('#include "dev.h"\n', '#include "dev.h"\n' + VH_INCLUDE),
            # dev_ifname (SIOCGIFNAME): hide a resolved VPN name.
            (
                "\tifr->ifr_name[IFNAMSIZ-1] = 0;\n"
                "\treturn netdev_get_name(net, ifr->ifr_name, ifr->ifr_ifindex);\n}",
                "\tint ret;\n\n"
                "\tifr->ifr_name[IFNAMSIZ-1] = 0;\n"
                "\tret = netdev_get_name(net, ifr->ifr_name, ifr->ifr_ifindex);\n"
                "\tif (ret == 0 &&\n"
                "\t    vpnhide_should_hide_ifname(ifr->ifr_name, VPNHIDE_HID_DEV_IOCTL))\n"
                "\t\treturn -ENODEV;\n"
                "\treturn ret;\n}",
            ),
            # dev_ifconf (SIOCGIFCONF enumeration): skip hidden devs.
            (
                "\tfor_each_netdev(net, dev) {\n\t\tif (!pos)",
                "\tfor_each_netdev(net, dev) {\n"
                "\t\tif (vpnhide_should_hide_dev(dev, VPNHIDE_HID_SOCK_IOCTL))\n"
                "\t\t\tcontinue;\n"
                "\t\tif (!pos)",
            ),
            # dev_ifsioc_locked (SIOCGIFFLAGS/MTU/HWADDR/INDEX/...): deny hidden.
            (
                "\tstruct net_device *dev = dev_get_by_name_rcu(net, ifr->ifr_name);\n"
                "\n\tif (!dev)\n\t\treturn -ENODEV;\n",
                "\tstruct net_device *dev = dev_get_by_name_rcu(net, ifr->ifr_name);\n"
                "\n\tif (!dev)\n\t\treturn -ENODEV;\n"
                "\n\tif (vpnhide_should_hide_dev(dev, VPNHIDE_HID_DEV_IOCTL))\n"
                "\t\treturn -ENODEV;\n",
            ),
        ],
        "net/core/rtnetlink.c": [
            (
                "#include <net/sock.h>\n#include <net/pkt_sched.h>\n",
                "#include <net/sock.h>\n" + VH_INCLUDE + "#include <net/pkt_sched.h>\n",
            ),
            (
                "\tstruct ifinfomsg *ifm;\n\tstruct nlmsghdr *nlh;\n"
                "\tstruct Qdisc *qdisc;\n\n\tASSERT_RTNL();\n",
                "\tstruct ifinfomsg *ifm;\n\tstruct nlmsghdr *nlh;\n"
                "\tstruct Qdisc *qdisc;\n\n"
                "\tif (dev &&\n"
                "\t    vpnhide_should_hide_dev(dev, VPNHIDE_HID_RTNL_FILL_IFINFO))\n"
                "\t\treturn 0;\n\n"
                "\tASSERT_RTNL();\n",
            ),
        ],
        "net/ipv4/devinet.c": [
            (
                "#include <net/ip_fib.h>\n#include <net/rtnetlink.h>\n",
                "#include <net/ip_fib.h>\n" + VH_INCLUDE + "#include <net/rtnetlink.h>\n",
            ),
            (
                "\tstruct nlmsghdr  *nlh;\n\tu32 preferred, valid;\n\n",
                "\tstruct nlmsghdr  *nlh;\n\tu32 preferred, valid;\n\n"
                "\tif (ifa->ifa_dev && ifa->ifa_dev->dev &&\n"
                "\t    vpnhide_should_hide_dev(ifa->ifa_dev->dev,\n"
                "\t\t\t\t    VPNHIDE_HID_INET_FILL_IFADDR))\n"
                "\t\treturn 0;\n\n",
            ),
        ],
        "net/ipv6/addrconf.c": [
            (
                "#include <net/addrconf.h>\n",
                "#include <net/addrconf.h>\n" + VH_INCLUDE,
            ),
            (
                "\tstruct nlmsghdr  *nlh;\n\tu32 preferred, valid;\n\n"
                "\tnlh = nlmsg_put(skb, args->portid, args->seq, args->event,\n",
                "\tstruct nlmsghdr  *nlh;\n\tu32 preferred, valid;\n\n"
                "\tif (ifa->idev && ifa->idev->dev &&\n"
                "\t    vpnhide_should_hide_dev(ifa->idev->dev,\n"
                "\t\t\t\t    VPNHIDE_HID_INET6_FILL_IFADDR))\n"
                "\t\treturn 0;\n\n"
                "\tnlh = nlmsg_put(skb, args->portid, args->seq, args->event,\n",
            ),
        ],
        "net/socket.c": [
            (
                "#include <net/sock.h>\n",
                "#include <net/sock.h>\n" + VH_INCLUDE,
            ),
            (
                "\tsockptr_t optval = USER_SOCKPTR(user_optval);\n"
                "\tchar *kernel_optval = NULL;\n"
                "\tint err, fput_needed;\n"
                "\tstruct socket *sock;\n",
                "\tsockptr_t optval = USER_SOCKPTR(user_optval);\n"
                "\tchar *kernel_optval = NULL;\n"
                "\tint err, fput_needed;\n"
                "\tstruct socket *sock;\n"
                "\tunion vpnhide_bind_snapshot vh_snap;\n",
            ),
            (
                "\tif (level == SOL_SOCKET && !sock_use_custom_sol_socket(sock))\n"
                "\t\terr = sock_setsockopt(sock, level, optname, optval, optlen);\n",
                "\tif (level == SOL_SOCKET) {\n"
                "\t\tenum vpnhide_bind_action vh_act =\n"
                "\t\t\tvpnhide_setsockopt_bind(sock->sk, optname, optval,\n"
                "\t\t\t\t\t\toptlen, &vh_snap);\n"
                "\t\tif (vh_act == VPNHIDE_BIND_DENY) {\n"
                "\t\t\terr = -ENODEV;\n"
                "\t\t\tgoto out_put;\n"
                "\t\t}\n"
                "\t\tif (vh_act == VPNHIDE_BIND_FAULT) {\n"
                "\t\t\terr = -EFAULT;\n"
                "\t\t\tgoto out_put;\n"
                "\t\t}\n"
                "\t\tif (vh_act == VPNHIDE_BIND_FROZEN)\n"
                "\t\t\toptval = KERNEL_SOCKPTR(&vh_snap);\n"
                "\t}\n\n"
                "\tif (level == SOL_SOCKET && !sock_use_custom_sol_socket(sock))\n"
                "\t\terr = sock_setsockopt(sock, level, optname, optval, optlen);\n",
            ),
        ],
        # --- filesystem path concealment (CONFIG_VPNHIDE_FS_HIDING) ----------
        "fs/namei.c": [
            (
                '#include <linux/uaccess.h>\n\n#include "internal.h"',
                '#include <linux/uaccess.h>\n' + VH_INCLUDE + '\n#include "internal.h"',
            ),
            # filename_lookup: hide a resolved VPN-iface path node.
            (
                "\tif (likely(!retval))\n"
                "\t\taudit_inode(name, path->dentry,\n"
                "\t\t\t    flags & LOOKUP_MOUNTPOINT ? AUDIT_INODE_NOEVAL : 0);\n"
                "\trestore_nameidata();\n"
                "\treturn retval;\n",
                "\tif (likely(!retval))\n"
                "\t\taudit_inode(name, path->dentry,\n"
                "\t\t\t    flags & LOOKUP_MOUNTPOINT ? AUDIT_INODE_NOEVAL : 0);\n"
                "\tif (!retval && vpnhide_should_hide_dentry(path->dentry)) {\n"
                "\t\tpath_put(path);\n"
                "\t\tpath->mnt = NULL;\n"
                "\t\tpath->dentry = NULL;\n"
                "\t\tretval = -ENOENT;\n"
                "\t}\n"
                "\trestore_nameidata();\n"
                "\treturn retval;\n",
            ),
            # do_filp_open: hide an opened VPN-iface path node.
            (
                "\trestore_nameidata();\n\treturn filp;\n}",
                "\tif (!IS_ERR(filp) &&\n"
                "\t    vpnhide_should_hide_dentry(filp->f_path.dentry)) {\n"
                "\t\tfput(filp);\n"
                "\t\tfilp = ERR_PTR(-ENOENT);\n"
                "\t}\n"
                "\trestore_nameidata();\n\treturn filp;\n}",
            ),
        ],
        "fs/stat.c": [
            (
                "#include <asm/unistd.h>\n",
                "#include <asm/unistd.h>\n" + VH_INCLUDE,
            ),
            (
                "\tretval = security_inode_getattr(path);\n"
                "\tif (retval)\n"
                "\t\treturn retval;\n"
                "\treturn vfs_getattr_nosec(path, stat, request_mask, query_flags);\n",
                "\tretval = security_inode_getattr(path);\n"
                "\tif (retval)\n"
                "\t\treturn retval;\n"
                "\tretval = vfs_getattr_nosec(path, stat, request_mask, query_flags);\n"
                "\tif (!retval && vpnhide_should_hide_dentry(path->dentry))\n"
                "\t\tretval = -ENOENT;\n"
                "\treturn retval;\n",
            ),
        ],
        "fs/readdir.c": [
            (
                "#include <linux/uaccess.h>\n",
                "#include <linux/uaccess.h>\n" + VH_INCLUDE,
            ),
            (
                "\tif (!IS_DEADDIR(inode)) {\n"
                "\t\tctx->pos = file->f_pos;\n"
                "\t\tif (shared)\n"
                "\t\t\tres = file->f_op->iterate_shared(file, ctx);\n"
                "\t\telse\n"
                "\t\t\tres = file->f_op->iterate(file, ctx);\n"
                "\t\tfile->f_pos = ctx->pos;\n",
                "\tif (!IS_DEADDIR(inode)) {\n"
                "\t\tbool vh_dir = vpnhide_readdir_begin(file, ctx);\n"
                "\t\tctx->pos = file->f_pos;\n"
                "\t\tif (shared)\n"
                "\t\t\tres = file->f_op->iterate_shared(file, ctx);\n"
                "\t\telse\n"
                "\t\t\tres = file->f_op->iterate(file, ctx);\n"
                "\t\tif (vh_dir)\n"
                "\t\t\tvpnhide_readdir_end(ctx);\n"
                "\t\tfile->f_pos = ctx->pos;\n",
            ),
        ],
        # --- route / policy-rule concealment --------------------------------
        "net/ipv4/fib_trie.c": [
            (
                '#include "fib_lookup.h"\n',
                VH_INCLUDE + '#include "fib_lookup.h"\n',
            ),
            (
                "\t\tunsigned int flags = fib_flag_trans(fa->fa_type, mask, fi);\n\n"
                "\t\tif ((fa->fa_type == RTN_BROADCAST) ||\n",
                "\t\tunsigned int flags = fib_flag_trans(fa->fa_type, mask, fi);\n\n"
                "\t\tif (vpnhide_hide_fib_route(fi))\n"
                "\t\t\tcontinue;\n\n"
                "\t\tif ((fa->fa_type == RTN_BROADCAST) ||\n",
            ),
        ],
        "net/ipv6/ip6_fib.c": [
            (
                "#include <net/ip6_route.h>\n",
                "#include <net/ip6_route.h>\n" + VH_INCLUDE,
            ),
            (
                "\tconst struct net_device *dev;\n\n\tif (rt->nh)\n",
                "\tconst struct net_device *dev;\n\n"
                "\tif (vpnhide_hide_fib6_route(rt)) {\n"
                "\t\titer->w.leaf = NULL;\n"
                "\t\treturn 0;\n"
                "\t}\n\n"
                "\tif (rt->nh)\n",
            ),
        ],
        "net/ipv4/fib_semantics.c": [
            (
                '#include "fib_lookup.h"\n',
                VH_INCLUDE + '#include "fib_lookup.h"\n',
            ),
            (
                "\tstruct rtmsg *rtm;\n\n"
                "\tnlh = nlmsg_put(skb, portid, seq, event, sizeof(*rtm), flags);\n",
                "\tstruct rtmsg *rtm;\n\n"
                "\tif (vpnhide_hide_fib_dump(fri))\n"
                "\t\treturn 0;\n\n"
                "\tnlh = nlmsg_put(skb, portid, seq, event, sizeof(*rtm), flags);\n",
            ),
        ],
        "net/ipv6/route.c": [
            (
                "#include <trace/events/fib6.h>\n",
                "#include <trace/events/fib6.h>\n" + VH_INCLUDE,
            ),
            (
                "\tlong expires = 0;\n\n"
                "\tnlh = nlmsg_put(skb, portid, seq, type, sizeof(*rtm), flags);\n",
                "\tlong expires = 0;\n\n"
                "\tif (vpnhide_hide_rt6(rt, dst))\n"
                "\t\treturn 0;\n\n"
                "\tnlh = nlmsg_put(skb, portid, seq, type, sizeof(*rtm), flags);\n",
            ),
        ],
        "net/core/fib_rules.c": [
            (
                "#include <linux/indirect_call_wrapper.h>\n",
                "#include <linux/indirect_call_wrapper.h>\n" + VH_INCLUDE,
            ),
            (
                "\tstruct fib_rule_hdr *frh;\n\n"
                "\tnlh = nlmsg_put(skb, pid, seq, type, sizeof(*frh), flags);\n",
                "\tstruct fib_rule_hdr *frh;\n\n"
                "\tif (vpnhide_hide_fib_rule(rule))\n"
                "\t\treturn 0;\n\n"
                "\tnlh = nlmsg_put(skb, pid, seq, type, sizeof(*frh), flags);\n",
            ),
        ],
    },
}

# android16-6.12: 26 of 30 call sites match android14-6.1 verbatim. Only four
# diverge, so derive the KMI by copying the 6.1 edits and replacing just those
# four entries (kept indices reuse the shared anchors):
#   - net/socket.c[1]: __sys_setsockopt's option dispatch moved into
#     do_sock_setsockopt(); the vh_snap decl must live in that function (the
#     interception at [2] already anchors on the dispatch line, which is there).
#   - inet_fill_ifaddr / inet6_fill_ifaddr: `const *ifa` + extra local decls.
#   - iterate_dir: 6.12 dropped the legacy .iterate leg (iterate_shared only).
_612 = copy.deepcopy(EDITS["android14-6.1"])
_612["net/socket.c"][1] = (
    "\tconst struct proto_ops *ops;\n"
    "\tchar *kernel_optval = NULL;\n"
    "\tint err;\n",
    "\tconst struct proto_ops *ops;\n"
    "\tchar *kernel_optval = NULL;\n"
    "\tint err;\n"
    "\tunion vpnhide_bind_snapshot vh_snap;\n",
)
_612["net/ipv4/devinet.c"][1] = (
    "\tu32 preferred, valid;\n\tu32 flags;\n\n"
    "\tnlh = nlmsg_put(skb, args->portid, args->seq, args->event, sizeof(*ifm),\n",
    "\tu32 preferred, valid;\n\tu32 flags;\n\n"
    "\tif (ifa->ifa_dev && ifa->ifa_dev->dev &&\n"
    "\t    vpnhide_should_hide_dev(ifa->ifa_dev->dev,\n"
    "\t\t\t\t    VPNHIDE_HID_INET_FILL_IFADDR))\n"
    "\t\treturn 0;\n\n"
    "\tnlh = nlmsg_put(skb, args->portid, args->seq, args->event, sizeof(*ifm),\n",
)
_612["net/ipv6/addrconf.c"][1] = (
    "\tstruct nlmsghdr *nlh;\n\tu32 preferred, valid;\n"
    "\tu32 flags, priority;\n\tu8 proto;\n\n"
    "\tnlh = nlmsg_put(skb, args->portid, args->seq, args->event,\n",
    "\tstruct nlmsghdr *nlh;\n\tu32 preferred, valid;\n"
    "\tu32 flags, priority;\n\tu8 proto;\n\n"
    "\tif (ifa->idev && ifa->idev->dev &&\n"
    "\t    vpnhide_should_hide_dev(ifa->idev->dev,\n"
    "\t\t\t\t    VPNHIDE_HID_INET6_FILL_IFADDR))\n"
    "\t\treturn 0;\n\n"
    "\tnlh = nlmsg_put(skb, args->portid, args->seq, args->event,\n",
)
_612["fs/readdir.c"][1] = (
    "\tif (!IS_DEADDIR(inode)) {\n"
    "\t\tctx->pos = file->f_pos;\n"
    "\t\tres = file->f_op->iterate_shared(file, ctx);\n"
    "\t\tfile->f_pos = ctx->pos;\n",
    "\tif (!IS_DEADDIR(inode)) {\n"
    "\t\tbool vh_dir = vpnhide_readdir_begin(file, ctx);\n"
    "\t\tctx->pos = file->f_pos;\n"
    "\t\tres = file->f_op->iterate_shared(file, ctx);\n"
    "\t\tif (vh_dir)\n"
    "\t\t\tvpnhide_readdir_end(ctx);\n"
    "\t\tfile->f_pos = ctx->pos;\n",
)
EDITS["android16-6.12"] = _612

# android15-6.6: derive from 6.1. Shares 6.12's do_sock_setsockopt refactor
# (setsockopt dispatch moved out of __sys_setsockopt), so the vh_snap decl uses
# the same anchor as 6.12; other overrides added below as divergences surface.
_66 = copy.deepcopy(EDITS["android14-6.1"])
_66["net/socket.c"][1] = _612["net/socket.c"][1]
_66["fs/readdir.c"][1] = _612["fs/readdir.c"][1]  # iterate_shared-only, as 6.12
EDITS["android15-6.6"] = _66

# android13-5.15: derive from 6.1. Older tree: net/core/dev.h did not exist yet,
# so the dev_ioctl.c include anchors after <net/wext.h>; more overrides below.
_515 = copy.deepcopy(EDITS["android14-6.1"])
_515["net/core/dev_ioctl.c"][0] = (
    "#include <net/wext.h>\n",
    "#include <net/wext.h>\n" + VH_INCLUDE,
)
EDITS["android13-5.15"] = _515

# android12-5.10: derive from 6.1. No net/core/dev.h (include after <net/wext.h>);
# dev_ifconf's loop declares `int done;` first; filename_lookup calls putname()
# between restore_nameidata() and the return, so anchor its tail on
# restore_nameidata() alone.
_510 = copy.deepcopy(EDITS["android14-6.1"])
_510["net/core/dev_ioctl.c"][0] = (
    "#include <net/wext.h>\n",
    "#include <net/wext.h>\n" + VH_INCLUDE,
)
_510["net/core/dev_ioctl.c"][2] = (
    "\tfor_each_netdev(net, dev) {\n\t\tint done;\n\t\tif (!pos)",
    "\tfor_each_netdev(net, dev) {\n"
    "\t\tint done;\n"
    "\t\tif (vpnhide_should_hide_dev(dev, VPNHIDE_HID_SOCK_IOCTL))\n"
    "\t\t\tcontinue;\n"
    "\t\tif (!pos)",
)
_510["fs/namei.c"][1] = (
    "\tif (likely(!retval))\n"
    "\t\taudit_inode(name, path->dentry,\n"
    "\t\t\t    flags & LOOKUP_MOUNTPOINT ? AUDIT_INODE_NOEVAL : 0);\n"
    "\trestore_nameidata();\n",
    "\tif (likely(!retval))\n"
    "\t\taudit_inode(name, path->dentry,\n"
    "\t\t\t    flags & LOOKUP_MOUNTPOINT ? AUDIT_INODE_NOEVAL : 0);\n"
    "\tif (!retval && vpnhide_should_hide_dentry(path->dentry)) {\n"
    "\t\tpath_put(path);\n"
    "\t\tpath->mnt = NULL;\n"
    "\t\tpath->dentry = NULL;\n"
    "\t\tretval = -ENOENT;\n"
    "\t}\n"
    "\trestore_nameidata();\n",
)
EDITS["android12-5.10"] = _510

# android11-5.4: predates sockptr_t (added in 5.9). __sys_setsockopt takes a bare
# `char __user *optval` with set_fs(), so bind uses vpnhide_setsockopt_bind_user()
# and freezes (anti-TOCTOU) by swapping optval to the kernel snapshot under
# set_fs(KERNEL_DS) — mirroring the kernel's own kernel_optval path — restoring
# set_fs after the dispatch (socket.c gains a 4th edit for that restore). Older
# call sites also diverge: no net/core/dev.h; dev_ifconf loops over NPROTO
# gifconf handlers; filename_lookup's audit_inode is the 3-arg form and putname()
# follows restore_nameidata(); fib_rules.c lacks <linux/indirect_call_wrapper.h>.
_54 = copy.deepcopy(EDITS["android14-6.1"])
_54["net/core/dev_ioctl.c"][0] = (
    "#include <net/wext.h>\n",
    "#include <net/wext.h>\n" + VH_INCLUDE,
)
_54["net/core/dev_ioctl.c"][2] = (
    "\tfor_each_netdev(net, dev) {\n\t\tfor (i = 0; i < NPROTO; i++) {",
    "\tfor_each_netdev(net, dev) {\n"
    "\t\tif (vpnhide_should_hide_dev(dev, VPNHIDE_HID_SOCK_IOCTL))\n"
    "\t\t\tcontinue;\n"
    "\t\tfor (i = 0; i < NPROTO; i++) {",
)
_54["net/socket.c"][1] = (
    "\tmm_segment_t oldfs = get_fs();\n"
    "\tchar *kernel_optval = NULL;\n"
    "\tint err, fput_needed;\n"
    "\tstruct socket *sock;\n",
    "\tmm_segment_t oldfs = get_fs();\n"
    "\tchar *kernel_optval = NULL;\n"
    "\tint err, fput_needed;\n"
    "\tstruct socket *sock;\n"
    "\tunion vpnhide_bind_snapshot vh_snap;\n"
    "\tbool vh_frozen = false;\n",
)
_54["net/socket.c"][2] = (
    "\t\tif (kernel_optval) {\n"
    "\t\t\tset_fs(KERNEL_DS);\n"
    "\t\t\toptval = (char __user __force *)kernel_optval;\n"
    "\t\t}\n",
    "\t\tif (level == SOL_SOCKET) {\n"
    "\t\t\tenum vpnhide_bind_action vh_act =\n"
    "\t\t\t\tvpnhide_setsockopt_bind_user(sock->sk, optname,\n"
    "\t\t\t\t\t\t\t     optval, optlen,\n"
    "\t\t\t\t\t\t\t     &vh_snap);\n"
    "\t\t\tif (vh_act == VPNHIDE_BIND_DENY) {\n"
    "\t\t\t\terr = -ENODEV;\n"
    "\t\t\t\tgoto out_put;\n"
    "\t\t\t}\n"
    "\t\t\tif (vh_act == VPNHIDE_BIND_FAULT) {\n"
    "\t\t\t\terr = -EFAULT;\n"
    "\t\t\t\tgoto out_put;\n"
    "\t\t\t}\n"
    "\t\t\tif (vh_act == VPNHIDE_BIND_FROZEN) {\n"
    "\t\t\t\tset_fs(KERNEL_DS);\n"
    "\t\t\t\toptval = (char __user __force *)&vh_snap;\n"
    "\t\t\t\tvh_frozen = true;\n"
    "\t\t\t}\n"
    "\t\t}\n\n"
    "\t\tif (kernel_optval) {\n"
    "\t\t\tset_fs(KERNEL_DS);\n"
    "\t\t\toptval = (char __user __force *)kernel_optval;\n"
    "\t\t}\n",
)
_54["net/socket.c"].append((
    "\t\tif (kernel_optval) {\n"
    "\t\t\tset_fs(oldfs);\n"
    "\t\t\tkfree(kernel_optval);\n"
    "\t\t}\n",
    "\t\tif (vh_frozen)\n"
    "\t\t\tset_fs(oldfs);\n\n"
    "\t\tif (kernel_optval) {\n"
    "\t\t\tset_fs(oldfs);\n"
    "\t\t\tkfree(kernel_optval);\n"
    "\t\t}\n",
))
_54["fs/namei.c"][1] = (
    "\tif (likely(!retval))\n"
    "\t\taudit_inode(name, path->dentry, 0);\n"
    "\trestore_nameidata();\n",
    "\tif (likely(!retval))\n"
    "\t\taudit_inode(name, path->dentry, 0);\n"
    "\tif (!retval && vpnhide_should_hide_dentry(path->dentry)) {\n"
    "\t\tpath_put(path);\n"
    "\t\tpath->mnt = NULL;\n"
    "\t\tpath->dentry = NULL;\n"
    "\t\tretval = -ENOENT;\n"
    "\t}\n"
    "\trestore_nameidata();\n",
)
_54["net/core/fib_rules.c"][0] = (
    "#include <net/fib_rules.h>\n",
    "#include <net/fib_rules.h>\n" + VH_INCLUDE,
)
# Pre-5.5 fib_dump_info(fi, dst, dst_len, …): no fib_rt_info, call the raw
# variant with the fields spread across its arguments.
_54["net/ipv4/fib_semantics.c"][1] = (
    "\tstruct rtmsg *rtm;\n\n"
    "\tnlh = nlmsg_put(skb, portid, seq, event, sizeof(*rtm), flags);\n",
    "\tstruct rtmsg *rtm;\n\n"
    "\tif (vpnhide_hide_fib_dump_raw(fi, dst, dst_len))\n"
    "\t\treturn 0;\n\n"
    "\tnlh = nlmsg_put(skb, portid, seq, event, sizeof(*rtm), flags);\n",
)
EDITS["android11-5.4"] = _54

# android10-4.19 / android10-4.14: derive from 5.4 (share the pre-sockptr bind,
# 3-arg audit_inode, NPROTO dev_ifconf loop, fib_dump_info raw args). Extra
# overrides added below as their older anchors surface.
_419 = copy.deepcopy(_54)
# rtnl_fill_ifinfo: no `struct Qdisc *qdisc;` local before ASSERT_RTNL yet.
_419["net/core/rtnetlink.c"][1] = (
    "\tstruct ifinfomsg *ifm;\n\tstruct nlmsghdr *nlh;\n\n\tASSERT_RTNL();\n",
    "\tstruct ifinfomsg *ifm;\n\tstruct nlmsghdr *nlh;\n\n"
    "\tif (dev &&\n"
    "\t    vpnhide_should_hide_dev(dev, VPNHIDE_HID_RTNL_FILL_IFINFO))\n"
    "\t\treturn 0;\n\n"
    "\tASSERT_RTNL();\n",
)
# inet6_fill_ifaddr: positional args (portid/seq/event), ifaddrmsg size inline.
_419["net/ipv6/addrconf.c"][1] = (
    "\tstruct nlmsghdr  *nlh;\n\tu32 preferred, valid;\n\n"
    "\tnlh = nlmsg_put(skb, portid, seq, event, sizeof(struct ifaddrmsg), flags);\n",
    "\tstruct nlmsghdr  *nlh;\n\tu32 preferred, valid;\n\n"
    "\tif (ifa->idev && ifa->idev->dev &&\n"
    "\t    vpnhide_should_hide_dev(ifa->idev->dev,\n"
    "\t\t\t\t    VPNHIDE_HID_INET6_FILL_IFADDR))\n"
    "\t\treturn 0;\n\n"
    "\tnlh = nlmsg_put(skb, portid, seq, event, sizeof(struct ifaddrmsg), flags);\n",
)
# __sys_setsockopt: pre-BPF/pre-kernel_optval — bare user pointer, no set_fs
# scaffolding, so declare our own oldfs and freeze around the dispatch. Anchor
# the decl on the optlen<0 guard to stay unique vs __sys_getsockopt.
_419["net/socket.c"][1] = (
    "\tint err, fput_needed;\n\tstruct socket *sock;\n\n"
    "\tif (optlen < 0)\n\t\treturn -EINVAL;\n",
    "\tint err, fput_needed;\n\tstruct socket *sock;\n"
    "\tunion vpnhide_bind_snapshot vh_snap;\n"
    "\tbool vh_frozen = false;\n"
    "\tmm_segment_t vh_oldfs;\n\n"
    "\tif (optlen < 0)\n\t\treturn -EINVAL;\n",
)
_419["net/socket.c"][2] = (
    "\t\tif (level == SOL_SOCKET)\n"
    "\t\t\terr =\n"
    "\t\t\t    sock_setsockopt(sock, level, optname, optval,\n"
    "\t\t\t\t\t    optlen);\n",
    "\t\tif (level == SOL_SOCKET) {\n"
    "\t\t\tenum vpnhide_bind_action vh_act =\n"
    "\t\t\t\tvpnhide_setsockopt_bind_user(sock->sk, optname,\n"
    "\t\t\t\t\t\t\t     optval, optlen,\n"
    "\t\t\t\t\t\t\t     &vh_snap);\n"
    "\t\t\tif (vh_act == VPNHIDE_BIND_DENY) {\n"
    "\t\t\t\terr = -ENODEV;\n"
    "\t\t\t\tgoto out_put;\n"
    "\t\t\t}\n"
    "\t\t\tif (vh_act == VPNHIDE_BIND_FAULT) {\n"
    "\t\t\t\terr = -EFAULT;\n"
    "\t\t\t\tgoto out_put;\n"
    "\t\t\t}\n"
    "\t\t\tif (vh_act == VPNHIDE_BIND_FROZEN) {\n"
    "\t\t\t\tvh_oldfs = get_fs();\n"
    "\t\t\t\tset_fs(KERNEL_DS);\n"
    "\t\t\t\toptval = (char __user __force *)&vh_snap;\n"
    "\t\t\t\tvh_frozen = true;\n"
    "\t\t\t}\n"
    "\t\t}\n\n"
    "\t\tif (level == SOL_SOCKET)\n"
    "\t\t\terr =\n"
    "\t\t\t    sock_setsockopt(sock, level, optname, optval,\n"
    "\t\t\t\t\t    optlen);\n",
)
_419["net/socket.c"][3] = (
    "\t\t\terr =\n"
    "\t\t\t    sock->ops->setsockopt(sock, level, optname, optval,\n"
    "\t\t\t\t\t\t  optlen);\n"
    "out_put:\n",
    "\t\t\terr =\n"
    "\t\t\t    sock->ops->setsockopt(sock, level, optname, optval,\n"
    "\t\t\t\t\t\t  optlen);\n\n"
    "\t\tif (vh_frozen)\n"
    "\t\t\tset_fs(vh_oldfs);\n"
    "out_put:\n",
)
# fs/namei.c: extra include (build_bug.h) before internal.h; filename_lookup's
# audit_inode takes `flags & LOOKUP_PARENT` here.
_419["fs/namei.c"][0] = (
    '#include <linux/build_bug.h>\n\n#include "internal.h"',
    '#include <linux/build_bug.h>\n' + VH_INCLUDE + '\n#include "internal.h"',
)
_419["fs/namei.c"][1] = (
    "\tif (likely(!retval))\n"
    "\t\taudit_inode(name, path->dentry, flags & LOOKUP_PARENT);\n"
    "\trestore_nameidata();\n",
    "\tif (likely(!retval))\n"
    "\t\taudit_inode(name, path->dentry, flags & LOOKUP_PARENT);\n"
    "\tif (!retval && vpnhide_should_hide_dentry(path->dentry)) {\n"
    "\t\tpath_put(path);\n"
    "\t\tpath->mnt = NULL;\n"
    "\t\tpath->dentry = NULL;\n"
    "\t\tretval = -ENOENT;\n"
    "\t}\n"
    "\trestore_nameidata();\n",
)
# ipv6_route_seq_show: pre-nexthop, no `if (rt->nh)` — anchor on the first
# seq_printf instead.
_419["net/ipv6/ip6_fib.c"][1] = (
    "\tconst struct net_device *dev;\n\n"
    '\tseq_printf(seq, "%pi6 %02x ", &rt->fib6_dst.addr, rt->fib6_dst.plen);\n',
    "\tconst struct net_device *dev;\n\n"
    "\tif (vpnhide_hide_fib6_route(rt)) {\n"
    "\t\titer->w.leaf = NULL;\n"
    "\t\treturn 0;\n"
    "\t}\n\n"
    '\tseq_printf(seq, "%pi6 %02x ", &rt->fib6_dst.addr, rt->fib6_dst.plen);\n',
)
EDITS["android10-4.19"] = _419

_414 = copy.deepcopy(_54)
EDITS["android10-4.14"] = _414


# Patch filename per source path: net/core/dev_ioctl.c -> net_core_dev_ioctl.c.patch
def patch_name(relpath: str) -> str:
    return relpath.replace("/", "_") + ".patch"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("tree", type=Path, help="clean kernel/common git checkout")
    ap.add_argument("--kmi", default="android14-6.1")
    ap.add_argument("--keep", action="store_true", help="leave edits in the tree")
    args = ap.parse_args()

    tree: Path = args.tree
    edits = EDITS.get(args.kmi)
    if edits is None:
        sys.exit(f"no edits defined for KMI {args.kmi!r}")
    if not (tree / ".git").exists():
        sys.exit(f"{tree} is not a git checkout (needed to diff/restore)")

    dirty = subprocess.run(
        ["git", "-C", str(tree), "status", "--porcelain"],
        capture_output=True, text=True, check=True,
    ).stdout.strip()
    if dirty:
        sys.exit(f"{tree} has uncommitted changes; refusing to edit it:\n{dirty}")

    touched = []
    for relpath, file_edits in edits.items():
        path = tree / relpath
        text = path.read_text()
        for anchor, replacement in file_edits:
            count = text.count(anchor)
            if count != 1:
                sys.exit(
                    f"{relpath}: anchor occurs {count}x (need exactly 1):\n"
                    f"----\n{anchor}\n----"
                )
            text = text.replace(anchor, replacement, 1)
        path.write_text(text)
        touched.append(relpath)
        print(f"[gen] edited {relpath} ({len(file_edits)} insertions)")

    out_dir = REPO_ROOT / "builtin" / "versions" / args.kmi
    out_dir.mkdir(parents=True, exist_ok=True)
    for relpath in touched:
        diff = subprocess.run(
            ["git", "-C", str(tree), "diff", "--", relpath],
            capture_output=True, text=True, check=True,
        ).stdout
        (out_dir / patch_name(relpath)).write_text(diff)
        print(f"[gen] wrote versions/{args.kmi}/{patch_name(relpath)}")

    if args.keep:
        print("[gen] --keep: leaving edits in the tree")
    else:
        subprocess.run(["git", "-C", str(tree), "checkout", "--", *touched], check=True)
        print("[gen] restored tree")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
