/* SPDX-License-Identifier: MIT */
#ifndef VPNHIDE_SOCKET_BIND_INDEX_H
#define VPNHIDE_SOCKET_BIND_INDEX_H

/* The resolved-index helpers reject a negative index before mutation. Let
 * their native capability checks run: old trees reject every unprivileged
 * bind, while backports may permit the first bind of an unbound socket.
 * Only translate the deliberate EINVAL, never a native EPERM. This avoids
 * guessing vendor policy or reading sk_bound_dev_if at a guessed offset.
 * Nested wrapper -> locked calls see -1 and leave it alone. */
static void socket_bind_index_before(hook_fargs4_t *fargs, void *udata)
{
	fargs->local.data0 = 0;
	if (!hook_active(VPNHIDE_HOOK_SOCKET_BIND_INTERFACE) ||
	    !socket_bind_ifindex_hidden((void *)fargs->arg0, (int)fargs->arg1))
		return;
	fargs->local.data0 = 1;
	fargs->local.data1 = fargs->arg1;
	fargs->arg1 = (uint64_t)-1;
}

static void socket_bind_index_after(hook_fargs4_t *fargs, void *udata)
{
	if (!fargs->local.data0)
		return;
	fargs->arg1 = fargs->local.data1;
	if ((int)fargs->ret == -22) {
		fargs->ret = VPNHIDE_ENODEV;
		record_hook_hit(VPNHIDE_HOOK_SOCKET_BIND_INTERFACE);
	}
}

static int install_socket_bind_index_hooks(void)
{
	static const struct {
		const char *name;
		int argno;
	} helpers[] = {
		/* LTO can inline the locked body into the public wrapper, while
		 * SO_BINDTOIFINDEX still calls the standalone locked symbol. */
		{ "sock_bindtoindex", 3 },
		{ "sock_bindtoindex_locked", 2 },
		{ "sock_setbindtodevice_locked", 2 },
	};
	unsigned int start = nr_hook_registrations;
	unsigned int i;

	for (i = 0; i < sizeof(helpers) / sizeof(helpers[0]); i++) {
		if (!lookup_fn(helpers[i].name))
			continue;
		if (!install_hook(helpers[i].name, helpers[i].argno,
				  (void *)socket_bind_index_before,
				  (void *)socket_bind_index_after,
				  VPNHIDE_HOOK_SOCKET_BIND_INTERFACE)) {
			rollback_hook_registrations(start);
			installed_hooks &= ~vpnhide_hook_bit(
				VPNHIDE_HOOK_SOCKET_BIND_INTERFACE);
			return 0;
		}
	}
	return nr_hook_registrations != start;
}

#endif
