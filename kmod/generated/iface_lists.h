/* AUTO-GENERATED from data/interfaces.toml — do not edit by hand. Regenerate with: python3 scripts/codegen-interfaces.py */
#ifndef VPNHIDE_GENERATED_IFACE_LISTS_H
#define VPNHIDE_GENERATED_IFACE_LISTS_H

static inline char vpnhide__lc(char c)
{
	return (c >= 'A' && c <= 'Z') ? (char)(c - 'A' + 'a') : c;
}

static inline int vpnhide_iface_starts_with_ci(const char *name, const char *prefix)
{
	int i;
	for (i = 0; prefix[i]; i++) {
		if (!name[i])
			return 0;
		if (vpnhide__lc(name[i]) != vpnhide__lc(prefix[i]))
			return 0;
	}
	return 1;
}

static inline int vpnhide_iface_starts_with_then_digits_ci(const char *name,
							   const char *prefix)
{
	int i = 0;
	if (!vpnhide_iface_starts_with_ci(name, prefix))
		return 0;
	while (prefix[i])
		i++;
	if (!name[i])
		return 0;
	for (; name[i]; i++)
		if (name[i] < '0' || name[i] > '9')
			return 0;
	return 1;
}

static inline int vpnhide_iface_equals_ci(const char *name, const char *other)
{
	int i;
	for (i = 0; other[i]; i++) {
		if (!name[i])
			return 0;
		if (vpnhide__lc(name[i]) != vpnhide__lc(other[i]))
			return 0;
	}
	return name[i] == '\0';
}

static inline int vpnhide_iface_contains_ci(const char *name, const char *needle)
{
	int i, j;
	if (!needle[0])
		return 1;
	for (i = 0; name[i]; i++) {
		for (j = 0; needle[j]; j++) {
			if (!name[i + j])
				return 0;
			if (vpnhide__lc(name[i + j]) != vpnhide__lc(needle[j]))
				break;
		}
		if (!needle[j])
			return 1;
	}
	return 0;
}

static inline int vpnhide_iface_is_vpn(const char *name)
{
	if (!name || !name[0])
		return 0;
	/* not a VPN: ipip fallback device tunl0 — matches the `tun` rule but is not a VPN */
	if (vpnhide_iface_starts_with_ci(name, "tunl"))
		return 0;
	/* not a VPN: ip_gre fallback device gre0 — matches the `gre` rule but is not a VPN */
	if (vpnhide_iface_equals_ci(name, "gre0"))
		return 0;
	/* not a VPN: ip_gre L2 fallback device gretap0 — matches the `gre` rule but is not a VPN */
	if (vpnhide_iface_equals_ci(name, "gretap0"))
		return 0;
	/* OpenVPN, WireGuard userspace, Tailscale, generic tunneling */
	if (vpnhide_iface_starts_with_ci(name, "tun"))
		return 1;
	/* OpenVPN bridged */
	if (vpnhide_iface_starts_with_ci(name, "tap"))
		return 1;
	/* WireGuard kernel */
	if (vpnhide_iface_starts_with_ci(name, "wg"))
		return 1;
	/* PPTP / L2TP PPP tunnels */
	if (vpnhide_iface_starts_with_ci(name, "ppp"))
		return 1;
	/* Android built-in IPsec VPN */
	if (vpnhide_iface_starts_with_ci(name, "ipsec"))
		return 1;
	/* kernel IPsec XFRM framework */
	if (vpnhide_iface_starts_with_ci(name, "xfrm"))
		return 1;
	/* Apple-style, rare on Android */
	if (vpnhide_iface_starts_with_ci(name, "utun"))
		return 1;
	/* L2TP */
	if (vpnhide_iface_starts_with_ci(name, "l2tp"))
		return 1;
	/* GRE tunnels */
	if (vpnhide_iface_starts_with_ci(name, "gre"))
		return 1;
	/* Tailscale native/root daemon tunnel (standard Android VpnService normally uses tun) */
	if (vpnhide_iface_starts_with_ci(name, "tailscale"))
		return 1;
	/* ZeroTier virtual Ethernet (zt followed by a network-derived interface id) */
	if (vpnhide_iface_starts_with_ci(name, "zt"))
		return 1;
	/* Hurricane Electric IPv6-in-IPv4 tunnel */
	if (vpnhide_iface_starts_with_ci(name, "he-ipv6"))
		return 1;
	/* catch-all for renamed clients (myvpn0, vpn-client, xvpn1, ...) */
	if (vpnhide_iface_contains_ci(name, "vpn"))
		return 1;
	/* Anonymous netdev / renamed tunnel using the kernel's default naming pattern (e.g. `ip link set tun0 name if33` from issue #86). Does NOT match `ifb<N>` — those are kernel intermediate-functional-block traffic-shaping ifaces (different shape: `if` + letter, not + digit). */
	if (vpnhide_iface_starts_with_then_digits_ci(name, "if"))
		return 1;
	return 0;
}

#endif /* VPNHIDE_GENERATED_IFACE_LISTS_H */
