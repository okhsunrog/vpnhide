/* SPDX-License-Identifier: MIT */
/* Host regression harness for the production pre/post hooks and installer.
 * Native fixtures model both legacy gates and the downstream relaxed gate,
 * including an inlined wrapper body which never enters the locked hook. */
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define VPNHIDE_HOOK_SOCKET_BIND_INTERFACE 25
#define VPNHIDE_ENODEV ((uint64_t) - 19)
typedef struct {
	uint64_t arg0, arg1, arg2, ret;
	struct {
		uint64_t data0, data1;
	} local;
} hook_fargs4_t;
static unsigned int nr_hook_registrations, installed_hooks;
static unsigned int present, fail, hits;
static int enabled;
static const char *names[] = { "sock_bindtoindex", "sock_bindtoindex_locked",
			       "sock_setbindtodevice_locked" };
static unsigned int vpnhide_hook_bit(int id)
{
	return 1u << id;
}
static int hook_active(int id)
{
	return enabled;
}
static int socket_bind_ifindex_hidden(void *sk, int index)
{
	/* 2 is a physical device, 7 a VPN, 99 an unknown positive index. */
	return index > 0 && index != 2;
}
static void record_hook_hit(int id)
{
	hits++;
}
static int helper_id(const char *name)
{
	for (int i = 0; i < 3; i++)
		if (!strcmp(names[i], name))
			return i;
	assert(0);
	return -1;
}
static unsigned long lookup_fn(const char *name)
{
	return present & (1u << helper_id(name));
}
static int install_hook(const char *name, int argno, void *before, void *after,
			int id)
{
	int i = helper_id(name);
	assert(argno == (i == 0 ? 3 : 2));
	assert(before && after);
	if (fail & (1u << i))
		return 0;
	nr_hook_registrations++;
	installed_hooks |= vpnhide_hook_bit(id);
	return 1;
}
static void rollback_hook_registrations(unsigned int start)
{
	nr_hook_registrations = start;
}
#include "kpm/socket_bind_index.h"

struct socket_fixture {
	int bound;
	int capable;
	int relaxed;
	int mutations;
};
static int native_bind(struct socket_fixture *sk, int index)
{
	if ((!sk->relaxed || sk->bound) && !sk->capable)
		return -1;
	if (index < 0)
		return -22;
	sk->bound = index;
	sk->mutations++;
	return 0;
}
static int intercepted_bind(struct socket_fixture *sk, int index, int nested)
{
	hook_fargs4_t args = { .arg0 = (uintptr_t)sk, .arg1 = (uint64_t)index };
	socket_bind_index_before(&args, NULL);
	/* nested=0 is the LTO-inlined body (also the direct locked entry).
	 * nested=1 is the ordinary wrapper calling the hooked locked helper. */
	args.ret = nested ? intercepted_bind(sk, (int)args.arg1, 0) :
			    native_bind(sk, (int)args.arg1);
	socket_bind_index_after(&args, NULL);
	assert((int)args.arg1 == index);
	return (int)args.ret;
}
int main(void)
{
	const int indices[] = { -1, 0, 2, 7, 99 };
	unsigned int cases = 0;
	for (unsigned int scenario = 0; scenario < 32; scenario++) {
		int relaxed = !!(scenario & 1);
		int cap = !!(scenario & 2);
		int bound = (scenario & 4) ? 2 : 0;
		int nested = !!(scenario & 8);
		enabled = !!(scenario & 16);
		for (unsigned int i = 0;
		     i < sizeof(indices) / sizeof(indices[0]); i++) {
			struct socket_fixture sk = { bound, cap, relaxed, 0 };
			struct socket_fixture reference = sk;
			int index = indices[i];
			int expected = native_bind(&reference, index);
			int denied = enabled && index > 2 && expected == 0;
			if (denied)
				expected = -19;
			hits = 0;
			assert(intercepted_bind(&sk, index, nested) ==
			       expected);
			assert(sk.bound == (denied ? bound : reference.bound));
			assert(sk.mutations ==
			       (denied ? 0 : reference.mutations));
			assert(hits == (unsigned int)denied);
			cases++;
		}
	}

	/* Every available path is required; a partial install rolls back while
	 * preserving unrelated registrations and their status bit. */
	for (present = 0; present < 8; present++)
		for (fail = 0; fail < 8; fail++) {
			nr_hook_registrations = 4;
			installed_hooks = 1;
			int expected = present && !(present & fail);
			assert(install_socket_bind_index_hooks() == expected);
			assert(nr_hook_registrations ==
			       4u + (expected ? __builtin_popcount(present) :
						0));
			assert(installed_hooks ==
			       (1u | (expected ? vpnhide_hook_bit(25) : 0)));
		}
	printf("PASS: %u bind cases and 64 installation cases\n", cases);
	return 0;
}
