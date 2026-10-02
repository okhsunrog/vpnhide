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
	bool got = vpnhide_iface_is_vpn(name);
	if (got != expected) {
		fprintf(stderr,
			"FAIL: vpnhide_iface_is_vpn(\"%s\") = %s, expected %s\n",
			name, got ? "true" : "false",
			expected ? "true" : "false");
		failures++;
	}
}

int main(void)
{
	check("tun0", true);
	check("tun", true);
	check("tun1234", true);
	check("tap0", true);
	check("wg0", true);
	check("wg-client", true);
	check("ppp0", true);
	check("ipsec0", true);
	check("xfrm0", true);
	check("utun3", true);
	check("l2tp0", true);
	check("gre1", true);
	check("tailscale0", true);
	check("ztyqb6mebi", true);
	check("zt0", true);
	check("he-ipv6", true);
	check("he-ipv6-1", true);
	check("TUN0", true);
	check("Wg99", true);
	check("MyVPN", true);
	check("custom_VPN_42", true);
	check("myvpn0", true);
	check("vpn", true);
	check("xvpn1", true);
	check("svpn0", true);
	check("lo", false);
	check("wlan0", false);
	check("wlan", false);
	check("rmnet0", false);
	check("rmnet_data0", false);
	check("rmnet_ipa0", false);
	check("eth0", false);
	check("ccmni0", false);
	check("seth_lte8", false);
	check("dummy0", false);
	check("bnep0", false);
	check("rndis0", false);
	check("tunl0", false);
	check("gre0", false);
	check("gretap0", false);
	check("gretap1", true);
	check("sit0", false);
	check("ip6tnl0", false);
	check("ip6gre0", false);
	check("erspan0", false);
	check("if33", true);
	check("if0", true);
	check("if99", true);
	check("ifb0", false);
	check("ifb1", false);
	check("if", false);
	check("if_inet6", false);
	check("", false);
	check("tunp", true);
	check("atun0", false);
	check("VPN", true);

	if (failures) {
		fprintf(stderr, "%d test(s) failed\n", failures);
		return 1;
	}
	printf("OK: 56 vectors passed\n");
	return 0;
}
