/* SPDX-License-Identifier: MIT */
/*
 * Diagnostic-thread RAW ownership, shared by kernel adapters. No UID table,
 * task_struct offsets, allocation, IO, busy-wait or wire parser.
 *
 * Root arms a UID/TID/nonce. The actual collector thread registers ITSELF,
 * transferring an opaque retained pid identity, without enabling RAW. Root
 * revalidates the worker/thread and activates that SAME object. There is never
 * a numeric PID lookup between authentication and activation: PID reuse cannot
 * retarget the grant. A repeated registration cannot replace the first object.
 *
 * Adapters authenticate root-only arm/activate/revoke separately. Registration
 * must use kernel current UID/TID, never caller-provided wire fields. current
 * obtains PIDTYPE_PID (zero on every supported kernel) and returns an owned pid
 * reference; put releases one reference. number operates on that reference.
 * Callbacks must not sleep. The maximum duration is adapter/wire-owned.
 *
 * Contention fails CLOSED (normal protection), including IRQ/preemption on the
 * owner's CPU. No caller waits for a preempted owner. A busy query is not an
 * inactive readback. Root confirms revoke/expiry before protected measurement.
 */
#ifndef VPNHIDE_DIAG_SCOPE_H
#define VPNHIDE_DIAG_SCOPE_H

enum vpnhide_diag_result {
	VPNHIDE_DIAG_OK,
	VPNHIDE_DIAG_BUSY,
	VPNHIDE_DIAG_INVALID,
	VPNHIDE_DIAG_STALE,
	VPNHIDE_DIAG_UNAVAILABLE,
};
enum vpnhide_diag_phase {
	VPNHIDE_DIAG_RELEASED,
	VPNHIDE_DIAG_ARMED,
	VPNHIDE_DIAG_REGISTERED,
	VPNHIDE_DIAG_ACTIVE,
};
struct vpnhide_diag_ops {
	void *context;
	void *(*current_identity)(void *context);
	unsigned int (*number)(void *context, void *identity);
	unsigned int (*uid)(void *context);
	void (*put)(void *context, void *identity);
	unsigned long long (*now)(void *context);
};
struct vpnhide_diag_scope {
	unsigned int gate;
	void *identity;
	unsigned int tid, uid;
	enum vpnhide_diag_phase phase;
	unsigned long long nonce, deadline, duration;
	/* Immutable after initialization. Root adapter supplies the wire limit. */
	unsigned long long max_lease;
};
struct vpnhide_diag_snapshot {
	enum vpnhide_diag_phase phase;
	unsigned int tid, uid;
	unsigned long long nonce, remaining;
};
static inline int vpnhide_diag_ops_valid(const struct vpnhide_diag_ops *ops)
{
	return ops && ops->current_identity && ops->number && ops->uid &&
	       ops->put && ops->now;
}
static inline int vpnhide_diag_try_lock(struct vpnhide_diag_scope *scope)
{
	unsigned int expected = 0;

	return __atomic_compare_exchange_n(&scope->gate, &expected, 1, 0,
					   __ATOMIC_ACQUIRE, __ATOMIC_RELAXED);
}
static inline void vpnhide_diag_unlock(struct vpnhide_diag_scope *scope)
{
	__atomic_store_n(&scope->gate, 0, __ATOMIC_RELEASE);
}
/* Caller owns gate; returns the reference to release AFTER unlocking. */
static inline void *vpnhide_diag_clear(struct vpnhide_diag_scope *scope)
{
	void *old = scope->identity;

	__atomic_store_n(&scope->identity, 0, __ATOMIC_RELAXED);
	scope->tid = scope->uid = 0;
	scope->deadline = scope->duration = 0;
	scope->phase = VPNHIDE_DIAG_RELEASED;
	/* Keep the nonce: replay must not resurrect a released grant. */
	return old;
}
/* The hot path never releases the lease's identity, only its temporary ref. */
static inline int vpnhide_diag_current(struct vpnhide_diag_scope *scope,
				       const struct vpnhide_diag_ops *ops)
{
	void *identity;
	int allowed = 0;

	if (!vpnhide_diag_ops_valid(ops) ||
	    !__atomic_load_n(&scope->identity, __ATOMIC_RELAXED) ||
	    !vpnhide_diag_try_lock(scope))
		return 0;
	if (scope->phase == VPNHIDE_DIAG_ACTIVE &&
	    ops->now(ops->context) < scope->deadline &&
	    ops->uid(ops->context) == scope->uid) {
		identity = ops->current_identity(ops->context);
		allowed = identity && identity == scope->identity;
		if (identity)
			ops->put(ops->context, identity);
	}
	vpnhide_diag_unlock(scope);
	return allowed;
}
/* Root-only adapter: this authorizes registration, never RAW access. */
static inline enum vpnhide_diag_result
vpnhide_diag_arm(struct vpnhide_diag_scope *scope,
		 const struct vpnhide_diag_ops *ops, unsigned int tid,
		 unsigned int uid, unsigned long long nonce,
		 unsigned long long duration)
{
	void *old = 0;
	unsigned long long now;
	enum vpnhide_diag_result result = VPNHIDE_DIAG_OK;

	if (!vpnhide_diag_ops_valid(ops) || !scope->max_lease)
		return VPNHIDE_DIAG_UNAVAILABLE;
	if (!tid || !uid || !nonce || !duration || duration > scope->max_lease)
		return VPNHIDE_DIAG_INVALID;
	if (!vpnhide_diag_try_lock(scope))
		return VPNHIDE_DIAG_BUSY;
	now = ops->now(ops->context);
	if (scope->max_lease > ~0ULL - now) {
		result = VPNHIDE_DIAG_INVALID;
	} else if (nonce == scope->nonce) {
		/* Neither RAW nor registration lifetime is renewed by replay. */
		if (scope->phase == VPNHIDE_DIAG_RELEASED ||
		    now >= scope->deadline || scope->tid != tid ||
		    scope->uid != uid || scope->duration != duration)
			result = VPNHIDE_DIAG_STALE;
	} else if (scope->phase != VPNHIDE_DIAG_RELEASED &&
		   now < scope->deadline) {
		result = VPNHIDE_DIAG_BUSY;
	} else {
		old = vpnhide_diag_clear(scope);
		scope->tid = tid;
		scope->uid = uid;
		scope->nonce = nonce;
		scope->duration = duration;
		scope->deadline = now + scope->max_lease;
		scope->phase = VPNHIDE_DIAG_ARMED;
	}
	vpnhide_diag_unlock(scope);
	if (old)
		ops->put(ops->context, old);
	return result;
}
/* Unprivileged self-registration; NEVER enables RAW without root activation. */
static inline enum vpnhide_diag_result
vpnhide_diag_register(struct vpnhide_diag_scope *scope,
		      const struct vpnhide_diag_ops *ops,
		      unsigned long long nonce)
{
	void *identity;
	enum vpnhide_diag_result result = VPNHIDE_DIAG_OK;

	if (!vpnhide_diag_ops_valid(ops))
		return VPNHIDE_DIAG_UNAVAILABLE;
	if (!vpnhide_diag_try_lock(scope))
		return VPNHIDE_DIAG_BUSY;
	if (!nonce || nonce != scope->nonce ||
	    scope->phase == VPNHIDE_DIAG_RELEASED ||
	    ops->now(ops->context) >= scope->deadline) {
		result = VPNHIDE_DIAG_STALE;
	} else if (ops->uid(ops->context) != scope->uid) {
		result = VPNHIDE_DIAG_INVALID;
	} else {
		identity = ops->current_identity(ops->context);
		if (!identity) {
			result = VPNHIDE_DIAG_UNAVAILABLE;
		} else if (ops->number(ops->context, identity) != scope->tid) {
			result = VPNHIDE_DIAG_INVALID;
		} else if (scope->identity && scope->identity != identity) {
			/* Same numeric TID/UID, different lifetime: never retarget. */
			result = VPNHIDE_DIAG_STALE;
		} else if (scope->phase == VPNHIDE_DIAG_ARMED) {
			__atomic_store_n(&scope->identity, identity,
					 __ATOMIC_RELAXED);
			identity = 0;
			scope->phase = VPNHIDE_DIAG_REGISTERED;
		}
		if (identity)
			ops->put(ops->context, identity);
	}
	vpnhide_diag_unlock(scope);
	return result;
}
/* Root-only adapter, after independent worker/thread revalidation. */
static inline enum vpnhide_diag_result
vpnhide_diag_activate(struct vpnhide_diag_scope *scope,
		      const struct vpnhide_diag_ops *ops,
		      unsigned long long nonce)
{
	unsigned long long now;
	enum vpnhide_diag_result result = VPNHIDE_DIAG_OK;

	if (!vpnhide_diag_ops_valid(ops))
		return VPNHIDE_DIAG_UNAVAILABLE;
	if (!vpnhide_diag_try_lock(scope))
		return VPNHIDE_DIAG_BUSY;
	now = ops->now(ops->context);
	if (!nonce || nonce != scope->nonce || !scope->identity ||
	    now >= scope->deadline) {
		result = VPNHIDE_DIAG_STALE;
	} else if (scope->phase == VPNHIDE_DIAG_REGISTERED) {
		if (scope->duration > ~0ULL - now) {
			result = VPNHIDE_DIAG_INVALID;
		} else {
			scope->deadline = now + scope->duration;
			scope->phase = VPNHIDE_DIAG_ACTIVE;
		}
	} else if (scope->phase != VPNHIDE_DIAG_ACTIVE) {
		result = VPNHIDE_DIAG_STALE;
	}
	vpnhide_diag_unlock(scope);
	return result;
}
static inline enum vpnhide_diag_result
vpnhide_diag_revoke(struct vpnhide_diag_scope *scope,
		    const struct vpnhide_diag_ops *ops,
		    unsigned long long nonce)
{
	void *old;

	if (!vpnhide_diag_ops_valid(ops))
		return VPNHIDE_DIAG_UNAVAILABLE;
	if (!nonce)
		return VPNHIDE_DIAG_INVALID;
	if (!vpnhide_diag_try_lock(scope))
		return VPNHIDE_DIAG_BUSY;
	if (nonce != scope->nonce) {
		vpnhide_diag_unlock(scope);
		return VPNHIDE_DIAG_STALE;
	}
	old = vpnhide_diag_clear(scope);
	vpnhide_diag_unlock(scope);
	if (old)
		ops->put(ops->context, old);
	return VPNHIDE_DIAG_OK;
}
/* Readback also reclaims expired identity; last nonce remains for dedupe. */
static inline enum vpnhide_diag_result
vpnhide_diag_snapshot(struct vpnhide_diag_scope *scope,
		      const struct vpnhide_diag_ops *ops,
		      struct vpnhide_diag_snapshot *snapshot)
{
	void *old = 0;
	unsigned long long now;

	if (!vpnhide_diag_ops_valid(ops))
		return VPNHIDE_DIAG_UNAVAILABLE;
	if (!vpnhide_diag_try_lock(scope))
		return VPNHIDE_DIAG_BUSY;
	now = ops->now(ops->context);
	if (scope->phase != VPNHIDE_DIAG_RELEASED && now >= scope->deadline)
		old = vpnhide_diag_clear(scope);
	snapshot->phase = scope->phase;
	snapshot->tid = scope->tid;
	snapshot->uid = scope->uid;
	snapshot->nonce = scope->nonce;
	snapshot->remaining = scope->phase != VPNHIDE_DIAG_RELEASED ?
				      scope->deadline - now :
				      0;
	vpnhide_diag_unlock(scope);
	if (old)
		ops->put(ops->context, old);
	return VPNHIDE_DIAG_OK;
}
#endif
