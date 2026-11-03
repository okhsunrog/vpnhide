/* SPDX-License-Identifier: MIT */
#include <assert.h>
#include <stdio.h>
#include <string.h>
#include "vpnhide_diag_wire.h"
struct identity {
	unsigned int tid, uid;
	int references;
};
struct world {
	unsigned long long time;
	struct identity *current;
};
static void *current(void *context)
{
	struct identity *identity = ((struct world *)context)->current;

	if (identity)
		identity->references++;
	return identity;
}
static unsigned int number(void *context, void *pointer)
{
	(void)context;
	return ((struct identity *)pointer)->tid;
}
static unsigned int uid(void *context)
{
	struct identity *identity = ((struct world *)context)->current;

	return identity ? identity->uid : 0;
}
static void put(void *context, void *pointer)
{
	struct identity *identity = pointer;

	(void)context;
	assert(identity->references > 0);
	identity->references--;
}
static unsigned long long now(void *context)
{
	return ((struct world *)context)->time;
}
static void wire_and_privileges(void)
{
	static const char *const malformed[] = {
		"v2 arm 42 10001 0000000000000007 5000\n",
		"v1 arm 0 10001 0000000000000007 5000\n",
		"v1 arm 2147483648 10001 0000000000000007 5000\n",
		"v1 arm 42 4294967296 0000000000000007 5000\n",
		"v1 arm 42 10001 0000000000000007 5001\n",
		"v1 arm 42 10001 0000000000000000 5000\n",
		"v1 arm 42 10001 0000000000000007 0\n",
		"v1 activate 000000000000000A\n",
		"v1 activate 7\n",
		"v1 revoke 0000000000000007",
		"v1 revoke 0000000000000007\n\n",
		"v1 register 0000000000000007 trailing\n",
	};
	struct identity collector = { 42, 10001, 0 }, root = { 1, 0, 0 };
	struct world world = { .current = &collector };
	struct vpnhide_diag_ops ops = { &world, current, number, uid, put, now };
	struct vpnhide_diag_scope scope = { .max_lease = 5000000000ULL };
	struct vpnhide_diag_command command;
	const char *arm = "v1 arm 42 10001 0123456789abcdef 5000\n";
	unsigned char registration[16] = { 0xef, 0xcd, 0xab, 0x89, 0x67, 0x45,
					   0x23, 0x01, 'V',  'H',  'R',	 'A',
					   'W',	 '1',  '!',  0 };
	unsigned int i;
	for (i = 0; i < sizeof(malformed) / sizeof(malformed[0]); i++)
		assert(!vpnhide_diag_parse_command(
			malformed[i], strlen(malformed[i]), &command));
	assert(vpnhide_diag_parse_command(arm, strlen(arm), &command));
	assert(vpnhide_diag_execute(&scope, &ops, &command) ==
	       VPNHIDE_DIAG_INVALID);
	world.current = &root;
	assert(vpnhide_diag_execute(&scope, &ops, &command) == VPNHIDE_DIAG_OK);
	world.current = &collector;
	registration[15] = 1;
	assert(vpnhide_diag_register_name(&scope, &ops, registration) ==
	       VPNHIDE_DIAG_INVALID);
	registration[15] = 0;
	assert(vpnhide_diag_register_name(&scope, &ops, registration) ==
	       VPNHIDE_DIAG_OK);
	assert(!vpnhide_diag_current(&scope, &ops));
	command.kind = VPNHIDE_DIAG_ACTIVATE;
	assert(vpnhide_diag_execute(&scope, &ops, &command) ==
	       VPNHIDE_DIAG_INVALID);
	world.current = &root;
	assert(vpnhide_diag_execute(&scope, &ops, &command) == VPNHIDE_DIAG_OK);
	world.current = &collector;
	assert(vpnhide_diag_current(&scope, &ops));
	command.kind = VPNHIDE_DIAG_REVOKE;
	assert(vpnhide_diag_execute(&scope, &ops, &command) ==
	       VPNHIDE_DIAG_INVALID);
	world.current = &root;
	assert(vpnhide_diag_execute(&scope, &ops, &command) == VPNHIDE_DIAG_OK);
	assert(!collector.references && !root.references);
}
int main(void)
{
	struct identity original = { 42, 10001, 0 }, reused = { 42, 10001, 0 };
	struct identity ordinary = { 43, 10001, 0 },
			other_uid = { 42, 10002, 0 };
	struct world world = { .current = &original };
	struct vpnhide_diag_ops ops = { &world, current, number, uid, put, now };
	struct vpnhide_diag_scope scope = { .max_lease = 5 };
	struct vpnhide_diag_snapshot snapshot;

	assert(vpnhide_diag_register(&scope, &ops, 7) == VPNHIDE_DIAG_STALE);
	assert(vpnhide_diag_arm(&scope, &ops, 42, 10001, 7, 5) ==
	       VPNHIDE_DIAG_OK);
	assert(!vpnhide_diag_current(&scope, &ops));
	assert(vpnhide_diag_activate(&scope, &ops, 7) == VPNHIDE_DIAG_STALE);
	world.current = &other_uid;
	assert(vpnhide_diag_register(&scope, &ops, 7) == VPNHIDE_DIAG_INVALID);
	world.current = &ordinary; /* same UID, different thread */
	assert(vpnhide_diag_register(&scope, &ops, 7) == VPNHIDE_DIAG_INVALID);
	assert(ordinary.references == 0);
	world.current = &original;
	assert(vpnhide_diag_register(&scope, &ops, 7) == VPNHIDE_DIAG_OK);
	assert(original.references == 1);
	assert(!vpnhide_diag_current(&scope,
				     &ops)); /* registration is NOT RAW */
	world.current =
		&reused; /* reuse before activation cannot replace identity */
	assert(vpnhide_diag_register(&scope, &ops, 7) == VPNHIDE_DIAG_STALE);
	assert(reused.references == 0);
	assert(vpnhide_diag_activate(&scope, &ops, 7) == VPNHIDE_DIAG_OK);
	assert(!vpnhide_diag_current(&scope, &ops));
	world.current = &original;
	assert(vpnhide_diag_current(&scope, &ops));
	world.current = &ordinary;
	assert(!vpnhide_diag_current(&scope, &ops));
	world.current = &other_uid;
	assert(!vpnhide_diag_current(&scope, &ops));
	world.current = &original;
	world.time = 4;
	assert(vpnhide_diag_activate(&scope, &ops, 7) == VPNHIDE_DIAG_OK);
	assert(vpnhide_diag_arm(&scope, &ops, 42, 10001, 7, 5) ==
	       VPNHIDE_DIAG_OK);
	assert(vpnhide_diag_snapshot(&scope, &ops, &snapshot) ==
	       VPNHIDE_DIAG_OK);
	assert(snapshot.phase == VPNHIDE_DIAG_ACTIVE &&
	       snapshot.remaining == 1);
	assert(vpnhide_diag_revoke(&scope, &ops, 8) == VPNHIDE_DIAG_STALE);

	scope.gate =
		1; /* preempted writer/IRQ: no spinning, no RAW, no fake readback */
	assert(!vpnhide_diag_current(&scope, &ops));
	assert(vpnhide_diag_arm(&scope, &ops, 42, 10001, 8, 5) ==
	       VPNHIDE_DIAG_BUSY);
	assert(vpnhide_diag_register(&scope, &ops, 7) == VPNHIDE_DIAG_BUSY);
	assert(vpnhide_diag_activate(&scope, &ops, 7) == VPNHIDE_DIAG_BUSY);
	assert(vpnhide_diag_revoke(&scope, &ops, 7) == VPNHIDE_DIAG_BUSY);
	assert(vpnhide_diag_snapshot(&scope, &ops, &snapshot) ==
	       VPNHIDE_DIAG_BUSY);
	assert(original.references == 1);
	scope.gate = 0;
	world.time = 5;
	assert(!vpnhide_diag_current(&scope, &ops));
	assert(vpnhide_diag_snapshot(&scope, &ops, &snapshot) ==
	       VPNHIDE_DIAG_OK);
	assert(snapshot.phase == VPNHIDE_DIAG_RELEASED && snapshot.nonce == 7);
	assert(original.references == 0);
	assert(vpnhide_diag_arm(&scope, &ops, 42, 10001, 7, 5) ==
	       VPNHIDE_DIAG_STALE);
	assert(vpnhide_diag_arm(&scope, &ops, 42, 10001, 8, 6) ==
	       VPNHIDE_DIAG_INVALID);
	assert(vpnhide_diag_arm(&scope, &ops, 42, 10001, 8, 5) ==
	       VPNHIDE_DIAG_OK);
	assert(vpnhide_diag_register(&scope, &ops, 8) == VPNHIDE_DIAG_OK);
	assert(vpnhide_diag_activate(&scope, &ops, 8) == VPNHIDE_DIAG_OK);
	assert(vpnhide_diag_revoke(&scope, &ops, 7) == VPNHIDE_DIAG_STALE);
	assert(vpnhide_diag_current(&scope, &ops));
	assert(vpnhide_diag_revoke(&scope, &ops, 8) == VPNHIDE_DIAG_OK);
	assert(original.references == 0 && reused.references == 0);
	world.time = ~0ULL - 1;
	assert(vpnhide_diag_arm(&scope, &ops, 42, 10001, 9, 5) ==
	       VPNHIDE_DIAG_INVALID);
	assert(original.references == 0);
	world.time = 10;
	assert(vpnhide_diag_arm(&scope, &ops, 42, 10001, 9, 5) ==
	       VPNHIDE_DIAG_OK);
	world.time = 15; /* registration handshake expires too */
	assert(vpnhide_diag_register(&scope, &ops, 9) == VPNHIDE_DIAG_STALE);
	assert(vpnhide_diag_activate(&scope, &ops, 9) == VPNHIDE_DIAG_STALE);
	assert(vpnhide_diag_snapshot(&scope, &ops, &snapshot) ==
	       VPNHIDE_DIAG_OK);
	assert(snapshot.phase == VPNHIDE_DIAG_RELEASED);
	wire_and_privileges();
	puts("diagnostic RAW self-registration/identity/expiry/dedup/contention PASS");
	return 0;
}
