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
struct vpnhide_diag_file {
	unsigned long long nonce;
};
static void *vpnhide_diag_pid_current(void *unused)
{
	(void)unused;
	return vpnhide_diag_getpid(current, PIDTYPE_PID);
}
static unsigned int vpnhide_diag_pid_number(void *unused, void *identity)
{
	pid_t number;

	(void)unused;
	number = vpnhide_diag_pidnr(identity);
	return number > 0 ? (unsigned int)number : 0;
}
static unsigned int vpnhide_diag_uid(void *unused)
{
	(void)unused;
	return __kuid_val(current_uid());
}
static void vpnhide_diag_pid_put(void *unused, void *identity)
{
	(void)unused;
	vpnhide_diag_putpid(identity);
}
static unsigned long long vpnhide_diag_now(void *unused)
{
	(void)unused;
	return (unsigned long long)ktime_to_ns(
		vpnhide_diag_clock(TK_OFFS_BOOT));
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
static int vpnhide_diag_raw_show(struct seq_file *seq, void *unused)
{
	struct vpnhide_diag_snapshot snapshot;
	unsigned long long remaining_ms;
	int error;

	(void)unused;
	if (__kuid_val(current_uid()) != 0)
		return -EACCES;
	error = vpnhide_diag_errno(vpnhide_diag_snapshot(
		&vpnhide_diag_state, &vpnhide_diag_callbacks, &snapshot));
	if (error)
		return error;
	/* Round UP: a live sub-millisecond lease must never claim it is expired. */
	remaining_ms = div_u64(snapshot.remaining + 999999ULL, 1000000U);
	seq_printf(seq,
		   VPNHIDE_DIAG_RAW_HEADER
		   "backend %u\nmax_ms %u\nphase %s\ntid %u\nuid %u\n"
		   "nonce %016llx\nremaining_ms %llu\n",
		   vpnhide_diag_backend, VPNHIDE_DIAG_RAW_MAX_MS,
		   vpnhide_diag_phase_name(snapshot.phase), snapshot.tid,
		   snapshot.uid, snapshot.nonce, remaining_ms);
	return 0;
}
static int vpnhide_diag_raw_open(struct inode *inode, struct file *file)
{
	struct vpnhide_diag_file *context;
	int error;

	(void)inode;
	if ((file->f_mode & FMODE_READ) && __kuid_val(current_uid()) != 0)
		return -EACCES;
	context = kzalloc(sizeof(*context), GFP_KERNEL);
	if (!context)
		return -ENOMEM;
	error = single_open(file, vpnhide_diag_raw_show, context);
	if (error)
		kfree(context);
	return error;
}
static int vpnhide_diag_raw_release(struct inode *inode, struct file *file)
{
	struct seq_file *seq = file->private_data;
	struct vpnhide_diag_file *context = seq->private;

	if (context->nonce)
		/* A contended release cannot spin in kernel context. The fixed TTL
		 * still revokes effective RAW; subsequent readback reclaims its ref.
		 * Only a matching nonce can clear the scope, never a newer owner.
		 */
		vpnhide_diag_revoke(&vpnhide_diag_state,
				    &vpnhide_diag_callbacks, context->nonce);
	kfree(context);
	return single_release(inode, file);
}
static ssize_t vpnhide_diag_raw_write(struct file *file,
				      const char __user *input, size_t bytes,
				      loff_t *position)
{
	struct seq_file *seq = file->private_data;
	struct vpnhide_diag_file *context = seq->private;
	struct vpnhide_diag_command command;
	char buffer[VPNHIDE_DIAG_RAW_COMMAND_BYTES];
	int error;

	(void)position;
	if (!bytes || bytes > sizeof(buffer))
		return -E2BIG;
	if (copy_from_user(buffer, input, bytes))
		return -EFAULT;
	if (!vpnhide_diag_parse_command(buffer, bytes, &command))
		return -EINVAL;
	if (__kuid_val(current_uid()) != 0)
		return -EPERM;
	error = vpnhide_diag_errno(vpnhide_diag_execute(
		&vpnhide_diag_state, &vpnhide_diag_callbacks, &command));
	if (error)
		return error;
	if (command.kind == VPNHIDE_DIAG_ARM)
		context->nonce = command.nonce;
	return bytes;
}
#if LINUX_VERSION_CODE >= KERNEL_VERSION(5, 6, 0)
static const struct proc_ops vpnhide_diag_raw_operations = {
	.proc_open = vpnhide_diag_raw_open,
	.proc_read = seq_read,
	.proc_write = vpnhide_diag_raw_write,
	.proc_lseek = seq_lseek,
	.proc_release = vpnhide_diag_raw_release,
};
#else
static const struct file_operations vpnhide_diag_raw_operations = {
	.owner = THIS_MODULE,
	.open = vpnhide_diag_raw_open,
	.read = seq_read,
	.write = vpnhide_diag_raw_write,
	.llseek = seq_lseek,
	.release = vpnhide_diag_raw_release,
};
#endif
static int vpnhide_diag_raw_init(unsigned int backend,
				 vpnhide_diag_getpid_fn getpid,
				 vpnhide_diag_putpid_fn putpid,
				 vpnhide_diag_pidnr_fn pidnr,
				 vpnhide_diag_clock_fn clock)
{
	if (!getpid || !putpid || !pidnr || !clock)
		return -EOPNOTSUPP;
	vpnhide_diag_getpid = getpid;
	vpnhide_diag_putpid = putpid;
	vpnhide_diag_pidnr = pidnr;
	vpnhide_diag_clock = clock;
	vpnhide_diag_backend = backend;
	vpnhide_diag_state.max_lease = VPNHIDE_DIAG_RAW_MAX_MS * 1000000ULL;
	/* Root-only control/readback. Collector self-registration uses the
	 * existing socket ioctl hook; no app proc permission is required.
	 */
	if (!proc_create(VPNHIDE_DIAG_RAW_NODE_NAME, 0600, NULL,
			 &vpnhide_diag_raw_operations)) {
		vpnhide_diag_state.max_lease = 0;
		return -ENOMEM;
	}
	return 0;
}
#endif
