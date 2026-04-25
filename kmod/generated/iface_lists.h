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

static inline bool vpnhide_iface_is_never_hide(const char *name)
{
	if (!name || !name[0])
		return false;
	/* 464XLAT CLAT shadow iface (v4-rmnet0, v4-wlan0, ...). Required on IPv6-only carriers (T-Mobile US, Reliance Jio, ...) — without it IPv4-only apps lose internet. Created by clatd, lives as ARPHRD_NONE TUN, easy to mistake for a VPN tunnel. AOSP source: external/android-clat. */
	if (vpnhide_iface_starts_with_then_any_ci(name, "v4-"))
		return true;
	/* OpenThread border router on Pixel 7+. Hard-coded in init.rc inside the com.android.tethering APEX (the same APEX that delivers VPN-related code). Used for Matter / smart-home Thread mesh, not connectivity for normal apps. */
	if (vpnhide_iface_equals_ci(name, "thread-wpan"))
		return true;
	/* IPv6-in-IPv4 tunnel placeholder (kmod: sit). ARPHRD_SIT=776. */
	if (vpnhide_iface_equals_ci(name, "sit0"))
		return true;
	/* IPv4 IPIP tunnel placeholder (kmod: ipip). ARPHRD_TUNNEL=768. */
	if (vpnhide_iface_equals_ci(name, "tunl0"))
		return true;
	/* IPv6 tunnel placeholder (kmod: ip6_tunnel). ARPHRD_TUNNEL6=769. */
	if (vpnhide_iface_equals_ci(name, "ip6tnl0"))
		return true;
	/* IPv4 VTI (IPsec) placeholder (kmod: ip_vti). ARPHRD_TUNNEL=768. */
	if (vpnhide_iface_equals_ci(name, "ip_vti0"))
		return true;
	/* IPv6 VTI (IPsec) placeholder (kmod: ip6_vti). ARPHRD_TUNNEL6=769. */
	if (vpnhide_iface_equals_ci(name, "ip6_vti0"))
		return true;
	/* GRE tunnel placeholder (kmod: ip_gre). ARPHRD_IPGRE=778. */
	if (vpnhide_iface_equals_ci(name, "gre0"))
		return true;
	/* Android system IPsec/XFRM placeholder. Created by the platform on stock Android (observed on Pixel 8 Pro / Android 16) as ARPHRD_NONE without a tun_flags attr — looks like a TUN VPN by ARPHRD alone, but is not. The numeric suffix is the system token; if vendor builds use a different one we'll add it explicitly rather than blanket-whitelisting ipsec* (which would let real IKEv2 VPNs created via IpSecTunnelInterface slip past). */
	if (vpnhide_iface_equals_ci(name, "ipsec250"))
		return true;
	return false;
}

#endif /* VPNHIDE_GENERATED_IFACE_LISTS_H */
