/* SPDX-License-Identifier: MIT */
/* Actual production count walker over an independent bounded memory fixture.
 * Golden field positions below come from compiler output or the actual pinned
 * Image instructions, not from the consumer's offset table. No kernel execution
 * or Android/compat-process acceptance is claimed by this host test.
 */
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include "generated/iface_lists.h"
#include "kpm/kver_offsets.h"

#define VPNHIDE_IFREQ_SZ 40
#define VPNHIDE_IFNAMSIZ 16
#define MODNAME "test"
#define logki(...) ((void)0)
static uint64_t memory[1024];
static unsigned int kver, reads, golden_ip, golden_head, golden_list,
	golden_label;
static const struct vpnhide_offsets *off;
static unsigned int golden_socket;
static int depth, lock_calls, unlock_calls, last_arity, sock_error;
static int read_helper, sock_helper, lock_helpers;
static const void *fault;
enum {
	FILE_AT = 0,
	SOCKET_AT = 64,
	SK_AT = 128,
	NET_AT = 256,
	DEV1 = 512,
	DEV2 = 1536,
	INDEV1 = 2560,
	INDEV2 = 2624,
	IFA1 = 2720,
	IFA2 = 2848,
	IFA3 = 2976,
	OTHER_NET = 4096
};
static char *at(unsigned int offset)
{
	return (char *)memory + offset;
}
static int ptr_is_kernel(const void *p)
{
	uintptr_t value = (uintptr_t)p, start = (uintptr_t)memory;
	return value >= start && value < start + sizeof(memory);
}
static int iface_is_vpn(const char *s)
{
	return vpnhide_iface_is_vpn(s);
}
static void pointer(unsigned int offset, void *value)
{
	memcpy(at(offset), &value, sizeof(value));
}
static void *socket_one(void *file)
{
	assert(file == at(FILE_AT));
	last_arity = 1;
	return sock_error ? 0 : at(SOCKET_AT);
}
static void *socket_two(void *file, int *error)
{
	assert(file == at(FILE_AT) && error);
	last_arity = 2;
	*error = sock_error;
	return sock_error ? 0 : at(SOCKET_AT);
}
static void lock(void)
{
	assert(!depth);
	depth = 1;
	lock_calls++;
}
static void unlock(void)
{
	assert(depth == 1);
	depth = 0;
	unlock_calls++;
}
static long read_kernel(void *dst, const void *src, unsigned long size)
{
	uintptr_t value = (uintptr_t)src, start = (uintptr_t)memory;
	reads++;
	if (!ptr_is_kernel(src) || size > sizeof(memory) ||
	    value - start > sizeof(memory) - size || src == fault)
		return -14;
	if (src != at(SOCKET_AT + golden_socket) && src != at(SK_AT + 48))
		assert(depth == 1);
	memcpy(dst, src, size);
	return 0;
}
static unsigned long kallsyms_lookup_name(const char *name)
{
	if (!strcmp(name, "sock_from_file"))
		return !sock_helper ?
			       0 :
			       (unsigned long)(kver < ((5 << 16) + (11 << 8)) ?
						       (void *)socket_two :
						       (void *)socket_one);
	if (!strcmp(name, "rtnl_lock"))
		return lock_helpers & 1 ? (unsigned long)lock : 0;
	if (!strcmp(name, "rtnl_unlock"))
		return lock_helpers & 2 ? (unsigned long)unlock : 0;
	if (!strcmp(name, "copy_from_kernel_nofault"))
		return read_helper == 1 ? (unsigned long)read_kernel : 0;
	if (!strcmp(name, "probe_kernel_read"))
		return read_helper == 2 ? (unsigned long)read_kernel : 0;
	assert(0);
	return 0;
}
#include "kpm/ifconf_count.h"

static void setup(unsigned int version)
{
	unsigned int list = golden_list;
	kver = version;
	off = vpnhide_select_offsets(version);
	memset(memory, 0, sizeof(memory));
	depth = lock_calls = unlock_calls = last_arity = sock_error = 0;
	reads = 0;
	fault = 0;
	read_helper = sock_helper = 1;
	lock_helpers = 3;
	golden_socket = version < (5 << 16) ? 32 : 24;
	pointer(SOCKET_AT + golden_socket, at(SK_AT));
	pointer(SK_AT + 48, at(NET_AT));
	pointer(NET_AT + golden_head, at(DEV1 + list));
	pointer(NET_AT + golden_head + 8, at(DEV2 + list));
	pointer(DEV1 + list, at(DEV2 + list));
	pointer(DEV1 + list + 8, at(NET_AT + golden_head));
	pointer(DEV2 + list, at(NET_AT + golden_head));
	pointer(DEV2 + list + 8, at(DEV1 + list));
	pointer(DEV1 + golden_ip, at(INDEV1));
	pointer(DEV2 + golden_ip, at(INDEV2));
	pointer(INDEV1, at(DEV1));
	pointer(INDEV2, at(DEV2));
	pointer(INDEV1 + 16, at(IFA1));
	pointer(INDEV2 + 16, at(IFA2));
	pointer(IFA1 + 24, at(INDEV1));
	pointer(IFA2 + 24, at(INDEV2));
	pointer(IFA3 + 24, at(INDEV2));
	pointer(IFA2 + 16, at(IFA3));
	strcpy(at(IFA1 + golden_label), "eth0");
	strcpy(at(IFA2 + golden_label), "vpn0");
	strcpy(at(IFA3 + golden_label), "vpn0:1");
	/* Another namespace has an empty list; the supplied socket owns NET_AT. */
	pointer(OTHER_NET + golden_head, at(OTHER_NET + golden_head));
	pointer(OTHER_NET + golden_head + 8, at(OTHER_NET + golden_head));
	resolve_ifconf_count_symbols();
}
static void refused(int original)
{
	int result = -777;
	assert(!ifconf_project_length(at(FILE_AT), original, &result));
	assert(result == -777 && !depth && lock_calls == unlock_calls);
}
static void controls(unsigned int version, unsigned int ip, unsigned int head,
		     unsigned int list, unsigned int label)
{
	golden_ip = ip;
	golden_head = head;
	golden_list = list;
	golden_label = label;
	int result = -777;
	setup(version);
	assert(ifconf_project_length(at(FILE_AT), 120, &result) &&
	       result == 40);
	assert(last_arity == (version < ((5 << 16) + (11 << 8)) ? 2 : 1));
	assert(lock_calls == 1 && unlock_calls == 1 && !depth);
	refused(80);
	refused(160);
	refused(119);
	refused(0);
	refused(-40);
	/* A borrowed socket selects its own namespace, not an ambient one. */
	pointer(SK_AT + 48, at(OTHER_NET));
	refused(120);
	setup(version);
	fault = at(IFA2 + golden_label);
	refused(120);
	setup(version);
	pointer(IFA2 + 24, at(INDEV1));
	refused(120);
	setup(version);
	pointer(INDEV1, at(DEV2));
	refused(120);
	setup(version);
	pointer(NET_AT + golden_head + 8, at(DEV1));
	refused(120);
	setup(version);
	pointer(IFA3 + 16, (void *)(uintptr_t)0x123);
	refused(120);
	setup(version);
	memset(at(IFA2 + golden_label), 'x', 16);
	refused(120);
	setup(version);
	strcpy(at(IFA2 + golden_label), "eth1");
	strcpy(at(IFA3 + golden_label), "eth2");
	refused(120);
	setup(version);
	read_helper = 0;
	resolve_ifconf_count_symbols();
	refused(120);
	setup(version);
	sock_helper = 0;
	resolve_ifconf_count_symbols();
	refused(120);
	for (int helpers = 0; helpers < 3; helpers++) {
		setup(version);
		lock_helpers = helpers;
		resolve_ifconf_count_symbols();
		refused(120);
	}
	setup(version);
	sock_error = -9;
	refused(120);
	setup(version);
	read_helper = 2;
	resolve_ifconf_count_symbols();
	assert(ifconf_project_length(at(FILE_AT), 120, &result) &&
	       result == 40);
	setup(version);
	pointer(IFA3 + 16, at(IFA2));
	refused(400000);
	assert(reads < 25000); /* cyclic input cannot strand a kernel caller */
}
int main(void)
{
	_Static_assert(sizeof(void *) == 8, "models the ARM64 kernel");
	controls((4 << 16) + (4 << 8), 696, 272, 80, 72);
	controls((4 << 16) + (9 << 8), 720, 288, 80, 72);
	controls((4 << 16) + (14 << 8), 704, 272, 80, 72);
	controls((4 << 16) + (19 << 8), 696, 272, 80, 76);
	controls((5 << 16) + (4 << 8), 720, 144, 80, 76);
	controls((5 << 16) + (10 << 8), 720, 144, 72, 76);
	controls((5 << 16) + (10 << 8), 728, 144, 72, 76);
	controls((5 << 16) + (15 << 8), 744, 144, 64, 76);
	controls((6 << 16) + (1 << 8), 720, 144, 64, 76);
	controls((6 << 16) + (6 << 8), 728, 144, 64, 76);
	controls((6 << 16) + (12 << 8), 968, 144, 352, 76);
	golden_head = 144;
	golden_list = 72;
	golden_label = 76;
	golden_ip = 720;
	setup((5 << 16) + (10 << 8));
	pointer(DEV1 + 728, at(INDEV1));
	pointer(DEV2 + 728, at(INDEV2));
	refused(120); /* two structurally valid layouts are still ambiguous */
	setup((5 << 16) + (12 << 8));
	refused(120);
	puts("KPM NULL-ifconf socket namespace/count/coherence/nofault/budget controls passed");
	return 0;
}
