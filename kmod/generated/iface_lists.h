/* AUTO-GENERATED from data/interfaces.toml — do not edit by hand. Regenerate with: python3 scripts/codegen-interfaces.py */
#ifndef VPNHIDE_GENERATED_IFACE_LISTS_H
#define VPNHIDE_GENERATED_IFACE_LISTS_H

#ifdef __KERNEL__
# include <linux/string.h>
# include <linux/ctype.h>
# include <linux/types.h>
#else
# include <ctype.h>
# include <stdbool.h>
# include <stddef.h>
# include <string.h>
#endif

static inline bool vpnhide_iface_starts_with_ci(
	const char *name, const char *prefix)
{
	size_t i;
	for (i = 0; prefix[i]; i++) {
		if (!name[i])
			return false;
		if (tolower((unsigned char)name[i]) !=
		    (unsigned char)prefix[i])
			return false;
	}
	return true;
}

static inline bool vpnhide_iface_starts_with_then_digits_ci(
	const char *name, const char *prefix)
{
	size_t i;
	if (!vpnhide_iface_starts_with_ci(name, prefix))
		return false;
	i = strlen(prefix);
	if (!name[i])
		return false;
	for (; name[i]; i++)
		if (name[i] < '0' || name[i] > '9')
			return false;
	return true;
}

static inline bool vpnhide_iface_starts_with_then_digits_optional_ci(
	const char *name, const char *prefix)
{
	size_t i;
	if (!vpnhide_iface_starts_with_ci(name, prefix))
		return false;
	for (i = strlen(prefix); name[i]; i++)
		if (name[i] < '0' || name[i] > '9')
			return false;
	return true;
}

static inline bool vpnhide_iface_starts_with_then_any_ci(
	const char *name, const char *prefix)
{
	if (!vpnhide_iface_starts_with_ci(name, prefix))
		return false;
	return name[strlen(prefix)] != '\0';
}

static inline bool vpnhide_iface_equals_ci(
	const char *name, const char *other)
{
	size_t i;
	for (i = 0; other[i]; i++) {
		if (!name[i])
			return false;
		if (tolower((unsigned char)name[i]) !=
		    (unsigned char)other[i])
			return false;
	}
	return name[i] == '\0';
}

static inline bool vpnhide_iface_contains_ci(
	const char *name, const char *needle)
{
	size_t nlen = strlen(needle);
	size_t i, j;
	if (nlen == 0)
		return true;
	for (i = 0; name[i]; i++) {
		for (j = 0; j < nlen; j++) {
			if (!name[i + j])
				return false;
			if (tolower((unsigned char)name[i + j]) !=
			    (unsigned char)needle[j])
				break;
		}
		if (j == nlen)
			return true;
	}
	return false;
}

static inline bool vpnhide_iface_is_vpn(const char *name)
{
	if (!name || !name[0])
		return false;
	/* OpenVPN, WireGuard userspace, Tailscale, generic tunneling */
	if (vpnhide_iface_starts_with_ci(name, "tun"))
		return true;
	/* OpenVPN bridged */
	if (vpnhide_iface_starts_with_ci(name, "tap"))
		return true;
	/* WireGuard kernel */
	if (vpnhide_iface_starts_with_ci(name, "wg"))
		return true;
	/* PPTP / L2TP PPP tunnels */
	if (vpnhide_iface_starts_with_ci(name, "ppp"))
		return true;
	/* Android built-in IPsec VPN */
	if (vpnhide_iface_starts_with_ci(name, "ipsec"))
		return true;
	/* kernel IPsec XFRM framework */
	if (vpnhide_iface_starts_with_ci(name, "xfrm"))
		return true;
	/* Apple-style, rare on Android */
	if (vpnhide_iface_starts_with_ci(name, "utun"))
		return true;
	/* L2TP */
	if (vpnhide_iface_starts_with_ci(name, "l2tp"))
		return true;
	/* GRE tunnels */
	if (vpnhide_iface_starts_with_ci(name, "gre"))
		return true;
	/* catch-all for renamed clients (myvpn0, vpn-client, xvpn1, ...) */
	if (vpnhide_iface_contains_ci(name, "vpn"))
		return true;
	return false;
}

#endif /* VPNHIDE_GENERATED_IFACE_LISTS_H */
