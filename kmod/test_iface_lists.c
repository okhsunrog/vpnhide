/* AUTO-GENERATED from data/interfaces.toml — do not edit by hand. Regenerate with: python3 scripts/codegen-interfaces.py */
/*
 * Userspace test driver for generated/iface_lists.h.
 * Build: gcc -O2 -Wall -Werror -o test_iface_lists test_iface_lists.c
 * Run: ./test_iface_lists  (exit 0 on success, 1 on failure)
 */

#include <stdbool.h>
#include <stdio.h>

#include "generated/iface_lists.h"

static int failures;

static void check(const char *name, bool expected)
{
	bool got = vpnhide_iface_is_never_hide(name);
	if (got != expected) {
		fprintf(stderr, "FAIL: vpnhide_iface_is_never_hide(\"%s\") = %s, expected %s\n",
			name, got ? "true" : "false", expected ? "true" : "false");
		failures++;
	}
}

int main(void)
{
	check("v4-rmnet0", true);
	check("v4-rmnet_data0", true);
	check("v4-wlan0", true);
	check("v4-x", true);
	check("thread-wpan", true);
	check("Thread-Wpan", true);
	check("v4-", false);
	check("v4", false);
	check("tun0", false);
	check("wg0", false);
	check("wlan0", false);
	check("thread-wpan-extra", false);
	check("if33", false);
	check("", false);

	if (failures) {
		fprintf(stderr, "%d test(s) failed\n", failures);
		return 1;
	}
	printf("OK: 14 vectors passed\n");
	return 0;
}
