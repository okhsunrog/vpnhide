/* SPDX-License-Identifier: MIT */
/* Linux proc adapter for builtin and .ko. KPM has a separate transport adapter.
 * Include once in the control-owner translation unit, never a hook callsite.
 */
#ifndef VPNHIDE_DIAG_PROC_H
#define VPNHIDE_DIAG_PROC_H
#include <linux/cred.h>
#include <linux/err.h>
#include <linux/fs.h>
#include <linux/math64.h>
#include <linux/module.h>
#include <linux/pid.h>
#include <linux/proc_fs.h>
#include <linux/sched.h>
#include <linux/seq_file.h>
#include <linux/slab.h>
#include <linux/timekeeping.h>
#include <linux/uaccess.h>
#include <linux/uidgid.h>
#include <linux/version.h>
#include "vpnhide_diag_wire.h"

typedef struct pid *(*vpnhide_diag_getpid_fn)(struct task_struct *,
					      enum pid_type);
typedef void (*vpnhide_diag_putpid_fn)(struct pid *);
typedef pid_t (*vpnhide_diag_pidnr_fn)(struct pid *);
typedef ktime_t (*vpnhide_diag_clock_fn)(enum tk_offsets);
static vpnhide_diag_getpid_fn vpnhide_diag_getpid;
static vpnhide_diag_putpid_fn vpnhide_diag_putpid;
static vpnhide_diag_pidnr_fn vpnhide_diag_pidnr;
static vpnhide_diag_clock_fn vpnhide_diag_clock;
static unsigned int vpnhide_diag_backend;
static struct vpnhide_diag_scope vpnhide_diag_state;
/* The .ko supplies raw kallsyms addresses, not canonical CFI jump-table
 * entries. Keep only these resolved calls behind non-inlined __nocfi boundaries,
 * like the module's other resolved VFS/socket helpers. The callbacks themselves
 * retain their typed callsites; barriers also prevent tail-call folding.
 */
static noinline __nocfi void *vpnhide_diag_pid_current(void *unused)
{
	struct pid *identity;

	(void)unused;
	identity = vpnhide_diag_getpid(current, PIDTYPE_PID);
	barrier();
	return identity;
}
static noinline __nocfi unsigned int vpnhide_diag_pid_number(void *unused,
							     void *identity)
{
	pid_t number;

	(void)unused;
	number = vpnhide_diag_pidnr(identity);
	barrier();
	return number > 0 ? (unsigned int)number : 0;
}
static unsigned int vpnhide_diag_uid(void *unused)
{
	(void)unused;
	return __kuid_val(current_uid());
}
static noinline __nocfi void vpnhide_diag_pid_put(void *unused, void *identity)
{
	(void)unused;
	vpnhide_diag_putpid(identity);
	barrier();
}
static noinline __nocfi unsigned long long vpnhide_diag_now(void *unused)
{
	ktime_t now;

	(void)unused;
	now = vpnhide_diag_clock(TK_OFFS_BOOT);
	barrier();
	return (unsigned long long)ktime_to_ns(now);
}
static const struct vpnhide_diag_ops vpnhide_diag_callbacks = {
	.current_identity = vpnhide_diag_pid_current,
	.number = vpnhide_diag_pid_number,
	.uid = vpnhide_diag_uid,
	.put = vpnhide_diag_pid_put,
	.now = vpnhide_diag_now,
};
static int vpnhide_diag_errno(enum vpnhide_diag_result result)
{
	switch (result) {
	case VPNHIDE_DIAG_OK:
		return 0;
	case VPNHIDE_DIAG_BUSY:
		return -EBUSY;
	case VPNHIDE_DIAG_INVALID:
		return -EINVAL;
	case VPNHIDE_DIAG_STALE:
		return -ESTALE;
	case VPNHIDE_DIAG_UNAVAILABLE:
		return -EOPNOTSUPP;
	}
	return -EINVAL;
}
static int vpnhide_diag_raw_current(void)
{
	/* Helpers are immutable after initialization, before the node is visible. */
	if (!vpnhide_diag_state.max_lease)
		return 0;
	return vpnhide_diag_current(&vpnhide_diag_state,
				    &vpnhide_diag_callbacks);
}
static int vpnhide_diag_raw_show(struct seq_file *seq)
{
	struct vpnhide_diag_snapshot snapshot;
	enum vpnhide_diag_result result;

	if (__kuid_val(current_uid()) != 0)
		return -EACCES;
	result = vpnhide_diag_snapshot(&vpnhide_diag_state,
				       &vpnhide_diag_callbacks, &snapshot);
	if (result != VPNHIDE_DIAG_OK) {
		seq_printf(seq, VPNHIDE_DIAG_RESULT_FMT, vpnhide_diag_backend,
			   vpnhide_diag_result_name(result));
		return 0;
	}
	seq_printf(seq, VPNHIDE_DIAG_STATE_FMT, vpnhide_diag_backend,
		   VPNHIDE_DIAG_RAW_MAX_MS,
		   vpnhide_diag_phase_name(snapshot.phase), snapshot.tid,
		   snapshot.uid, snapshot.nonce,
		   div_u64(snapshot.remaining + 999999ULL, 1000000U),
		   snapshot.renewal);
	return 0;
}

static ssize_t vpnhide_diag_raw_write(const char *buffer, size_t bytes)
{
	struct vpnhide_diag_command command;
	int error;

	if (__kuid_val(current_uid()) != 0)
		return -EPERM;
	if (!vpnhide_diag_parse_command(buffer, bytes, &command))
		return -EINVAL;
	error = vpnhide_diag_errno(vpnhide_diag_execute(
		&vpnhide_diag_state, &vpnhide_diag_callbacks, &command));
	return error ? error : (ssize_t)bytes;
}

static int vpnhide_diag_raw_init(unsigned int backend,
				 vpnhide_diag_getpid_fn getpid,
				 vpnhide_diag_putpid_fn putpid,
				 vpnhide_diag_pidnr_fn pidnr,
				 vpnhide_diag_clock_fn clock)
{
	vpnhide_diag_backend = backend;
	if (!getpid || !putpid || !pidnr || !clock)
		return -EOPNOTSUPP;
	vpnhide_diag_getpid = getpid;
	vpnhide_diag_putpid = putpid;
	vpnhide_diag_pidnr = pidnr;
	vpnhide_diag_clock = clock;
	WRITE_ONCE(vpnhide_diag_state.max_lease,
		   VPNHIDE_DIAG_RAW_MAX_MS * 1000000ULL);
	return 0;
}
#endif
