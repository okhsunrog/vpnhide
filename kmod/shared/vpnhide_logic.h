/* SPDX-License-Identifier: MIT */
/*
 * vpnhide shared filtering logic — backend-agnostic, FREESTANDING.
 *
 * This header is included by BOTH native backends:
 *   - the kretprobe `.ko` (compiled with the real Linux kernel headers), and
 *   - the KernelPatch KPM (compiled `-nostdinc` against KernelPatch's
 *     own headers).
 *
 * Therefore it must depend on NOTHING: no libc, no kernel headers, no
 * `memmove`/`strlen`. It operates only on caller-provided buffers + a
 * caller-provided interface-name matcher. Keep it that way — the moment
 * this header `#include`s anything, one of the two backends stops
 * compiling.
 *
 * Both native backends call these implementations directly so their route
 * filtering and configuration parsing stay behaviorally aligned.
 */
#ifndef VPNHIDE_SHARED_LOGIC_H
#define VPNHIDE_SHARED_LOGIC_H

#ifndef VPNHIDE_IFNAMSIZ
#define VPNHIDE_IFNAMSIZ 16
#endif

/* Matcher: returns non-zero if `ifname` (NUL-terminated, <= IFNAMSIZ) is a
 * VPN interface. Backends pass `vpnhide_iface_is_vpn` from the generated
 * `iface_lists.h` (single source of truth: data/interfaces.toml). */
typedef int (*vpnhide_match_fn)(const char *ifname);

/* Which whitespace-delimited field carries the interface name. */
enum vpnhide_iface_field {
	VPNHIDE_FIELD_FIRST =
		0, /* /proc/net/route        — first tab field   */
	VPNHIDE_FIELD_LAST =
		1, /* /proc/net/ipv6_route    — last ws field      */
};

/* ====================================================================== */
/*  Public-host-route concealment predicates (shared by .ko + KPM).        */
/*                                                                          */
/*  A VPN client pins a host-route to the server's PUBLIC address on the    */
/*  PHYSICAL uplink so tunnel packets can escape. That route leaks the      */
/*  server's address through a routing-table dump even when the tun iface   */
/*  is hidden, so both backends hide it. The byte/address logic below is    */
/*  pure (no kernel types); each backend supplies the kernel-struct reads.  */
/* ====================================================================== */

/* Case-insensitive ASCII prefix test. Self-contained (freestanding): the
 * generated iface_lists.h has an equivalent, but this header must not depend
 * on it (or on libc). */
static inline int vpnhide_str_starts_with_ci(const char *s, const char *prefix)
{
	unsigned int i;

	if (!s || !prefix)
		return 0;
	for (i = 0; prefix[i]; i++) {
		char a = s[i];
		char b = prefix[i];

		if (a >= 'A' && a <= 'Z')
			a = (char)(a + ('a' - 'A'));
		if (b >= 'A' && b <= 'Z')
			b = (char)(b + ('a' - 'A'));
		if (a != b)
			return 0;
	}
	return 1;
}

/* A physical (non-tunnel) uplink by name prefix: the only device the
 * host-route concealment fires on (a public route pinned to a tun is left to
 * the normal VPN-iface matcher). rmnet/ccmni/ccemni/seth are cellular, wlan
 * Wi-Fi, eth wired/tethering. */
static inline int vpnhide_iface_is_physical(const char *name)
{
	if (!name)
		return 0;
	return vpnhide_str_starts_with_ci(name, "rmnet") ||
	       vpnhide_str_starts_with_ci(name, "wlan") ||
	       vpnhide_str_starts_with_ci(name, "eth") ||
	       vpnhide_str_starts_with_ci(name, "ccmni") ||
	       vpnhide_str_starts_with_ci(name, "ccemni") ||
	       vpnhide_str_starts_with_ci(name, "seth");
}

/* True if `cmd` is a get-interface-by-name ioctl: the whole SIOCGIF* family
 * from SIOCGIFNAME (0x8910) through SIOCGIFMAP (0x8970), each of which takes a
 * `struct ifreq` with the interface name in `ifr_name`. A backend that gates
 * dev_ioctl on the opcode (the KPM does; the zygisk libc hook carries the same
 * range in Rust) MUST span the full range: `SIOCGIFINDEX` (0x8933, the ioctl
 * `if_nametoindex()` issues), `SIOCGIFTXQLEN` (0x8942), and `SIOCGIFMAP`
 * (0x8970) all sit above the obvious FLAGS/MTU/HWADDR cluster, and a too-narrow
 * ceiling leaks VPN presence (a non-zero `if_nametoindex("vpn0")` is a positive
 * fingerprint). SIOCGIFCONF (0x8912) is handled on a separate path (sock_ioctl),
 * so its membership here is harmless. The `.ko` needs no opcode gate at all — it
 * filters on the returned `ifr_name` — but the predicate stays freestanding and
 * available to any opcode-gated backend. */
static inline int vpnhide_ioctl_is_get_by_name(unsigned long cmd)
{
	return cmd >= 0x8910 && cmd <= 0x8970;
}

/* Public-IPv4 test on the 4 network-order bytes of an address (be[0] is the
 * first octet, so this is endianness-explicit — no host byte-swap needed).
 * Rejects 0/8, 10/8, 127/8, 224/4+, 100.64/10, 169.254/16, 172.16/12,
 * 192.168/16, and the 192.0.0/24, 192.0.2/24, 198.18/15, 198.51.100/24,
 * 203.0.113/24 special blocks. */
static inline int vpnhide_is_public_ipv4(const unsigned char *be)
{
	unsigned int a, b, c;

	if (!be)
		return 0;
	a = be[0];
	b = be[1];
	c = be[2];
	if (a == 0 || a == 10 || a == 127 || a >= 224)
		return 0;
	if (a == 100 && b >= 64 && b <= 127)
		return 0;
	if (a == 169 && b == 254)
		return 0;
	if (a == 172 && b >= 16 && b <= 31)
		return 0;
	if (a == 192 && b == 168)
		return 0;
	if (a == 192 && b == 0 && c == 0)
		return 0;
	if (a == 192 && b == 0 && c == 2)
		return 0;
	if (a == 198 && (b == 18 || b == 19))
		return 0;
	if (a == 198 && b == 51 && c == 100)
		return 0;
	if (a == 203 && b == 0 && c == 113)
		return 0;
	return 1;
}

/* Public-IPv6 test on the 16 address bytes (network order). Global unicast
 * 2000::/3 only — excludes ::/::1 (unspec/loopback), fe80::/10 (link-local),
 * fc00::/7 (ULA), ff00::/8 (multicast), and 2001:db8::/32 (documentation). */
static inline int vpnhide_is_public_ipv6(const unsigned char *addr)
{
	if (!addr)
		return 0;
	if ((addr[0] & 0xe0) != 0x20)
		return 0;
	if (addr[0] == 0x20 && addr[1] == 0x01 && addr[2] == 0x0d &&
	    addr[3] == 0xb8)
		return 0;
	return 1;
}

/*
 * Compact VPN lines out of a /proc/net seq-file buffer in place.
 *
 * `buf`   : seq_file buffer base.
 * `start` : byte offset where THIS show()-call's output began (everything
 *           before it belongs to earlier entries and must be preserved).
 * `count` : current end-of-content offset (seq_file->count).
 * `field` : where the iface name is on each line.
 * `match` : VPN-name predicate.
 *
 * Returns the new content length (caller writes it back to seq_file->count).
 *
 * Compaction only ever moves bytes DOWN (dst <= src), so the overlapping
 * copy is safe with a plain forward byte loop — no memmove dependency.
 */
static inline unsigned long
vpnhide_compact_seq_lines(char *buf, unsigned long start, unsigned long count,
			  enum vpnhide_iface_field field,
			  vpnhide_match_fn match)
{
	unsigned long src = start;
	unsigned long dst = start;
	char ifname[VPNHIDE_IFNAMSIZ];

	if (!buf || count <= start || !match)
		return count;

	while (src < count) {
		/* Find end of the current line (past the '\n', or EOF). */
		unsigned long nl = src;
		unsigned long line_end;
		unsigned long line_len;
		int hide;

		while (nl < count && buf[nl] != '\n')
			nl++;
		line_end = (nl < count) ? nl + 1 : count;
		line_len = line_end - src;

		/* Extract the interface name for this line. */
		if (field == VPNHIDE_FIELD_FIRST) {
			unsigned long j = 0;
			while (j < (unsigned long)(VPNHIDE_IFNAMSIZ - 1) &&
			       src + j < line_end && buf[src + j] != '\t' &&
			       buf[src + j] != '\n') {
				ifname[j] = buf[src + j];
				j++;
			}
			ifname[j] = '\0';
		} else {
			/* Last whitespace-delimited field. Trim trailing
			 * newline / CR / spaces / tabs, then walk back to the
			 * preceding separator. */
			unsigned long fe = line_end;
			unsigned long fs;
			unsigned long j = 0;

			while (fe > src &&
			       (buf[fe - 1] == '\n' || buf[fe - 1] == '\r' ||
				buf[fe - 1] == ' ' || buf[fe - 1] == '\t'))
				fe--;
			fs = fe;
			while (fs > src && buf[fs - 1] != ' ' &&
			       buf[fs - 1] != '\t')
				fs--;
			while (j < (unsigned long)(VPNHIDE_IFNAMSIZ - 1) &&
			       fs + j < fe) {
				ifname[j] = buf[fs + j];
				j++;
			}
			ifname[j] = '\0';
		}

		hide = (ifname[0] != '\0') && match(ifname);

		if (hide) {
			src = line_end; /* drop this line */
			continue;
		}

		if (dst != src) {
			unsigned long k;
			for (k = 0; k < line_len; k++)
				buf[dst + k] =
					buf[src + k]; /* dst<=src: safe */
		}
		dst += line_len;
		src = line_end;
	}

	return dst;
}

/* ====================================================================== */
/*  Control & stats protocol (docs/protocol.md) — wire parse + serialise  */
/*                                                                        */
/*  Freestanding, shared VERBATIM by the `.ko` and the KPM (the two       */
/*  native kernel backends). The app (Kotlin) and Zygisk (Rust) carry the */
/*  same grammar in their own languages; parity is held by golden vectors */
/*  + differential tests (protocol §8), NOT by shared code. This file is  */
/*  the C side of that parity — keep it byte-faithful to the spec, and pin */
/*  every change with a vector in shared/protocol_vectors.tsv.            */
/*                                                                        */
/*  Roles (protocol §1.3): a kernel backend PARSES config and EMITS       */
/*  stats/status. So C implements vpnhide_parse_config + vpnhide_format_* */
/*  only; Rust activators serialise config and the Kotlin app parses      */
/*  telemetry. peek_kind + clamp_to_line serve KPM ctl0 transport (§7).   */
/* ====================================================================== */

/*
 * TWO protocols share this lexical core and the hook-id registry (§5); they are
 * versioned apart because their readers ship apart (§4.2).
 *
 *   control    the `config` payload, activator -> backend. Its reader is the
 *              module shipped in the SAME flashable zip as the activator that
 *              writes it, so the two never meet across a version boundary and
 *              a bump costs nothing.
 *   telemetry  the `stats` + `status` payloads, backend -> app. One
 *              /proc/vpnhide_ctl read returns both back to back: one reader,
 *              one delivery, so one version. That reader is the APP, which
 *              updates independently of the modules — a bump here breaks an
 *              older APK's dashboard and diagnostics.
 */
#define VPNHIDE_CONTROL_VERSION 2
#define VPNHIDE_TELEMETRY_VERSION 1

/* Self-documenting banner a read endpoint prepends to its snapshot (§OPEN-7).
 * It is a `#` comment line, ignored by every parser (§4.1), so it costs
 * nothing structurally and lets an agent learn the replace-whole semantics
 * from a single `cat`. Prepended at the read site, never by format_*  — the
 * golden vectors test the bare format. */
#define VPNHIDE_READ_BANNER \
	"# vpnhide v1 — a WRITE replaces ENTIRE state; this read is status+stats\n"

enum vpnhide_kind {
	VPNHIDE_KIND_INVALID = -1,
	VPNHIDE_KIND_CONFIG = 0,
	VPNHIDE_KIND_STATS = 1,
	VPNHIDE_KIND_STATUS = 2,
};

/* one parsed uid-to-hookmask entry from a grouped `targets` record (§4.3) */
struct vpnhide_target {
	unsigned int uid;
	unsigned int hookmask;
};

/* one sparse `<hook_id>:<count>` stats cell for a given uid (§4.3). The
 * producer groups consecutive entries by uid; format_stats emits one line per
 * uid run. count is u64 cumulative-since-load (OPEN-3). */
struct vpnhide_stat_entry {
	unsigned int uid;
	unsigned int hook_id;
	unsigned long long count;
};

/* a `status` snapshot (§4.3): backend id, kernel version, installed-hook mask,
 * dominant error code. All hex on the wire. */
struct vpnhide_status {
	unsigned int backend;
	unsigned int kver;
	unsigned int hooks;
	unsigned int error;
};

/* --- lexical helpers (§4.1) ------------------------------------------- */

static inline int vpnhide_is_sep(char c)
{
	return c == ' ' || c == '\t';
}

static inline int vpnhide_is_ascii(char c)
{
	unsigned char u = (unsigned char)c;

	return u >= 0x20 && u <= 0x7e;
}

/*
 * Carve the next line out of [*i, len). On success returns 1 and sets
 * [*ls,*le) to the line content with a trailing CR stripped (CRLF → LF, §4.1),
 * advancing *i to the start of the following line. Returns 0 at end of buffer.
 */
static inline int vpnhide_next_line(const char *b, unsigned long len,
				    unsigned long *i, unsigned long *ls,
				    unsigned long *le)
{
	unsigned long s, e;

	if (*i >= len)
		return 0;
	s = *i;
	e = s;
	while (e < len && b[e] != '\n')
		e++;
	*i = (e < len) ? e + 1 : len;
	if (e > s && b[e - 1] == '\r')
		e--;
	*ls = s;
	*le = e;
	return 1;
}

/*
 * Classify a line [ls,le): returns 1 if it is significant (carries a token and
 * is not a `#` comment), setting *cs to the first non-whitespace offset. Blank
 * and comment lines return 0. *ascii is set to 0 if any byte is non-ASCII
 * (§4.1: such a line is rejected by the caller).
 */
static inline int vpnhide_line_significant(const char *b, unsigned long ls,
					   unsigned long le, unsigned long *cs,
					   int *ascii)
{
	unsigned long p;

	*ascii = 1;
	for (p = ls; p < le; p++)
		if (!vpnhide_is_ascii(b[p]) &&
		    b[p] != '\t') /* tab is a sep, §4.1 */
			*ascii = 0;
	p = ls;
	while (p < le && vpnhide_is_sep(b[p]))
		p++;
	*cs = p;
	if (p >= le)
		return 0; /* blank */
	if (b[p] == '#')
		return 0; /* comment */
	return 1;
}

/*
 * Next whitespace-delimited token within [*i, end). Returns 1 and sets
 * [*ts,*te); advances *i past the token. Runs of separators collapse to one
 * (§4.1). Returns 0 when no token remains.
 */
static inline int vpnhide_next_token(const char *b, unsigned long *i,
				     unsigned long end, unsigned long *ts,
				     unsigned long *te)
{
	unsigned long p = *i;

	while (p < end && vpnhide_is_sep(b[p]))
		p++;
	if (p >= end) {
		*i = p;
		return 0;
	}
	*ts = p;
	while (p < end && !vpnhide_is_sep(b[p]))
		p++;
	*te = p;
	*i = p;
	return 1;
}

/* True if token [ts,te) equals the NUL-terminated literal `lit`. */
static inline int vpnhide_tok_eq(const char *b, unsigned long ts,
				 unsigned long te, const char *lit)
{
	unsigned long p = ts;

	while (p < te && *lit && b[p] == *lit) {
		p++;
		lit++;
	}
	return p == te && *lit == '\0';
}

/* Parse a bare decimal token (the header version only, §4.2). 1 on success. */
static inline int vpnhide_tok_decimal(const char *b, unsigned long ts,
				      unsigned long te, unsigned long *out)
{
	unsigned long v = 0, p, digit;

	if (ts >= te)
		return 0;
	for (p = ts; p < te; p++) {
		if (b[p] < '0' || b[p] > '9')
			return 0;
		digit = (unsigned long)(b[p] - '0');
		/* Reject values that would overflow unsigned long instead of
		 * wrapping mod 2^64 and slipping past the version fuse — same
		 * contract as the hex parser below. */
		if (v > (~0UL - digit) / 10u)
			return 0;
		v = v * 10u + digit;
	}
	*out = v;
	return 1;
}

/*
 * Bare hex, no `0x`: the numeric primitive of the v2 `config` payload (§4.4).
 * The prefix costs two bytes on every number, and the KPM copies a whole config
 * through a fixed 1024-byte buffer — so the prefix is a direct tax on how many
 * apps fit, buying nothing at that width. `stats`/`status` keep the prefixed
 * form below: they are still version 1.
 */
static inline int vpnhide_tok_hex_bare(const char *b, unsigned long ts,
				       unsigned long te, int bits,
				       unsigned long long *out)
{
	unsigned long long v = 0;
	unsigned long long max = (bits >= 64) ? 0xffffffffffffffffULL :
						0xffffffffULL;
	unsigned long p;

	if (te <= ts) /* need at least one digit */
		return 0;
	for (p = ts; p < te; p++) {
		unsigned int d;
		char c = b[p];

		if (c >= '0' && c <= '9')
			d = (unsigned int)(c - '0');
		else if (c >= 'a' && c <= 'f')
			d = (unsigned int)(c - 'a' + 10);
		else if (c >= 'A' && c <= 'F')
			d = (unsigned int)(c - 'A' + 10);
		else
			return 0;
		if (v > (max - d) / 16ULL) /* width overflow */
			return 0;
		v = v * 16ULL + d;
	}
	*out = v;
	return 1;
}

/*
 * Parse the one numeric primitive (§4.4): `0x` (mandatory) + >=1 hex digit,
 * any case on read. `bits` is the field width (32 or 64); a value that
 * overflows it → reject (return 0), never wrap or saturate. 1 on success.
 */
static inline int vpnhide_tok_hex(const char *b, unsigned long ts,
				  unsigned long te, int bits,
				  unsigned long long *out)
{
	unsigned long long v = 0;
	unsigned long long max = (bits >= 64) ? 0xffffffffffffffffULL :
						0xffffffffULL;
	unsigned long p = ts;

	if (te - ts < 3) /* need "0x" + at least one digit */
		return 0;
	if (b[p] != '0')
		return 0;
	p++;
	if (b[p] != 'x' && b[p] != 'X')
		return 0;
	p++;
	for (; p < te; p++) {
		unsigned int d;
		char c = b[p];

		if (c >= '0' && c <= '9')
			d = (unsigned int)(c - '0');
		else if (c >= 'a' && c <= 'f')
			d = (unsigned int)(c - 'a' + 10);
		else if (c >= 'A' && c <= 'F')
			d = (unsigned int)(c - 'A' + 10);
		else
			return 0;
		if (v > (max - d) / 16ULL) /* width overflow */
			return 0;
		v = v * 16ULL + d;
	}
	*out = v;
	return 1;
}

/* --- header (§4.2) --------------------------------------------------- */

/*
 * Parse the mandatory header line. Returns the kind, and (if `next` is given)
 * sets *next to the offset of the first record line. Rejects the whole payload
 * (VPNHIDE_KIND_INVALID) when the header is missing/malformed or its version
 * is not the current version for that kind (§3 version fuse).
 */
static inline enum vpnhide_kind
vpnhide_parse_header(const char *b, unsigned long len, unsigned long *next)
{
	unsigned long i = 0, ls, le, cs, p, ts, te, ver;
	int ascii;

	while (vpnhide_next_line(b, len, &i, &ls, &le)) {
		if (!vpnhide_line_significant(b, ls, le, &cs, &ascii)) {
			if (next)
				*next = i;
			continue;
		}
		/* first significant line == the mandatory header */
		if (next)
			*next = i;
		if (!ascii)
			return VPNHIDE_KIND_INVALID;
		p = cs;
		if (!vpnhide_next_token(b, &p, le, &ts, &te) ||
		    !vpnhide_tok_eq(b, ts, te, "vpnhide"))
			return VPNHIDE_KIND_INVALID;
		if (!vpnhide_next_token(b, &p, le, &ts, &te) ||
		    !vpnhide_tok_decimal(b, ts, te, &ver))
			return VPNHIDE_KIND_INVALID;
		if (!vpnhide_next_token(b, &p, le, &ts, &te))
			return VPNHIDE_KIND_INVALID;
		/* The fuse is per kind: a version only means something once we
		 * know which payload it labels. */
		if (vpnhide_tok_eq(b, ts, te, "config"))
			return ver != VPNHIDE_CONTROL_VERSION ?
				       VPNHIDE_KIND_INVALID :
				       VPNHIDE_KIND_CONFIG;
		if (vpnhide_tok_eq(b, ts, te, "stats"))
			return ver != VPNHIDE_TELEMETRY_VERSION ?
				       VPNHIDE_KIND_INVALID :
				       VPNHIDE_KIND_STATS;
		if (vpnhide_tok_eq(b, ts, te, "status"))
			return ver != VPNHIDE_TELEMETRY_VERSION ?
				       VPNHIDE_KIND_INVALID :
				       VPNHIDE_KIND_STATUS;
		return VPNHIDE_KIND_INVALID;
	}
	if (next)
		*next = i;
	return VPNHIDE_KIND_INVALID; /* no significant line ⇒ no header */
}

/* Cheap kind sniff for transport dispatch (KPM ctl0, §7.1). */
static inline enum vpnhide_kind vpnhide_peek_kind(const char *b,
						  unsigned long len)
{
	return vpnhide_parse_header(b, len, 0);
}

/* Parse the optional KPM stats-page cursor (§7.2). Unknown records retain the
 * normal extensibility rule; a present `after` record is strict and unique so
 * a malformed/repeated cursor can never silently select the wrong page. */
static inline int vpnhide_parse_stats_after(const char *b, unsigned long len,
					    unsigned int *after)
{
	unsigned long i, ls, le, cs, p, ts, te;
	unsigned long long value;
	int ascii, seen = 0;

	if (vpnhide_parse_header(b, len, &i) != VPNHIDE_KIND_STATS)
		return 0;
	*after = 0;
	while (vpnhide_next_line(b, len, &i, &ls, &le)) {
		if (!vpnhide_line_significant(b, ls, le, &cs, &ascii) || !ascii)
			continue;
		p = cs;
		if (!vpnhide_next_token(b, &p, le, &ts, &te) ||
		    !vpnhide_tok_eq(b, ts, te, "after"))
			continue;
		if (seen || !vpnhide_next_token(b, &p, le, &ts, &te) ||
		    !vpnhide_tok_hex(b, ts, te, 32, &value) ||
		    vpnhide_next_token(b, &p, le, &ts, &te))
			return 0;
		*after = (unsigned int)value;
		seen = 1;
	}
	return 1;
}

/* --- config parse (§4.3) --------------------------------------------- */

/*
 * Insert keeping out[0..n) sorted ascending by uid; duplicate uid ⇒ last wins
 * (§4.3). Returns 0 when the set is already full and the uid is new — the
 * caller rejects the payload rather than dropping the overflow, because a
 * config the backend cannot hold in full means the producer and the backend
 * disagree about the ceiling, and applying a partial target set silently is
 * exactly the failure mode we are removing.
 *
 * Sorted-on-parse is a contract, not an optimisation: it is what lets the
 * hot path binary-search this array on every hooked call, and it makes the
 * parsed form independent of the order the producer grouped uids in. The
 * insertion is O(n) memmove per new uid, run once per config write with n
 * bounded by the caller's array — irrelevant next to the hot-path win.
 */
static inline int vpnhide_target_set(struct vpnhide_target *out, int *n,
				     int max, unsigned int uid,
				     unsigned int hookmask)
{
	int lo = 0, hi = *n - 1, pos, k;

	while (lo <= hi) {
		int mid = lo + (hi - lo) / 2;

		if (out[mid].uid == uid) {
			out[mid].hookmask = hookmask;
			return 1;
		}
		if (out[mid].uid < uid)
			lo = mid + 1;
		else
			hi = mid - 1;
	}
	if (*n >= max)
		return 0;
	pos = lo;
	for (k = *n; k > pos; k--)
		out[k] = out[k - 1];
	out[pos].uid = uid;
	out[pos].hookmask = hookmask;
	(*n)++;
	return 1;
}

/*
 * Parse a `config` payload into out[] (capacity `max`), sorted ascending by
 * uid. Returns the target count, or -1 if the payload is rejected whole.
 *
 * Rejected whole: bad/missing header, version too new, a non-config kind, a
 * missing or mismatched `end` record, a malformed uid inside a `targets`
 * group, or more uids than `max`. Unknown keywords are still skipped (§4.5),
 * so the grammar stays extensible.
 *
 * The `end <count>` fuse is what makes a truncated payload fail closed. The
 * KPM transport copies a config through a fixed 1024-byte buffer and truncates
 * silently on overflow, so "apply whatever prefix arrived" would mean quietly
 * running with a partial target set — the exact failure this rejects.
 *
 * *debug receives 0/1 if a `debug` line is present and is left UNCHANGED
 * otherwise (the caller seeds it with the live value so "absent ⇒ unchanged-
 * from-default", §4.3). *default_mask receives the `default` record's mask, or
 * 0 when absent: the hookmask for any uid NOT in out[]. Zero makes out[] the
 * set to act on; non-zero inverts that and makes out[] the exception list.
 */
static inline int vpnhide_parse_config(const char *b, unsigned long len,
				       struct vpnhide_target *out, int max,
				       int *debug, unsigned int *default_mask)
{
	unsigned long i, ls, le, cs, p, ts, te;
	int ascii, n = 0, have_end = 0;
	unsigned long long uids_seen = 0, declared = 0;
	enum vpnhide_kind k = vpnhide_parse_header(b, len, &i);

	if (k != VPNHIDE_KIND_CONFIG)
		return -1;
	if (default_mask)
		*default_mask = 0;

	while (vpnhide_next_line(b, len, &i, &ls, &le)) {
		if (!vpnhide_line_significant(b, ls, le, &cs, &ascii) || !ascii)
			continue;
		p = cs;
		if (!vpnhide_next_token(b, &p, le, &ts, &te))
			continue;
		if (vpnhide_tok_eq(b, ts, te, "debug")) {
			if (!vpnhide_next_token(b, &p, le, &ts, &te))
				continue;
			if (vpnhide_tok_eq(b, ts, te, "0")) {
				if (debug)
					*debug = 0;
			} else if (vpnhide_tok_eq(b, ts, te, "1")) {
				if (debug)
					*debug = 1;
			}
			/* any other flag value ⇒ malformed, skip */
		} else if (vpnhide_tok_eq(b, ts, te, "default")) {
			unsigned long long dm;

			if (!vpnhide_next_token(b, &p, le, &ts, &te) ||
			    !vpnhide_tok_hex_bare(b, ts, te, 32, &dm))
				continue;
			if (default_mask)
				*default_mask = (unsigned int)dm;
		} else if (vpnhide_tok_eq(b, ts, te, "targets")) {
			unsigned long long hm, uid;

			if (!vpnhide_next_token(b, &p, le, &ts, &te) ||
			    !vpnhide_tok_hex_bare(b, ts, te, 32, &hm))
				continue; /* malformed group head ⇒ skip line */
			while (vpnhide_next_token(b, &p, le, &ts, &te)) {
				/* A malformed uid inside a group is NOT skipped:
				 * it would desync the `end` count, so reject. */
				if (!vpnhide_tok_hex_bare(b, ts, te, 32, &uid))
					return -1;
				uids_seen++;
				if (!vpnhide_target_set(out, &n, max,
							(unsigned int)uid,
							(unsigned int)hm))
					return -1; /* over the backend ceiling */
			}
		} else if (vpnhide_tok_eq(b, ts, te, "end")) {
			if (!vpnhide_next_token(b, &p, le, &ts, &te) ||
			    !vpnhide_tok_hex_bare(b, ts, te, 32, &declared))
				continue;
			have_end = 1;
		}
		/* unknown first token ⇒ skip the line (§4.5) */
	}
	if (!have_end || declared != uids_seen)
		return -1;
	return n;
}

/* --- serialise (§4.3/§4.4) ------------------------------------------- */

/*
 * Bounded output cursor. `len` always counts the FULL intended length
 * (snprintf semantics) so the caller can detect overflow as len > cap; bytes
 * are only ever written while within cap, so it can never overrun `p`.
 */
struct vpnhide_buf {
	char *p;
	unsigned long cap;
	unsigned long len;
};

static inline void vpnhide_putc(struct vpnhide_buf *b, char c)
{
	if (b->len < b->cap)
		b->p[b->len] = c;
	b->len++;
}

static inline void vpnhide_puts(struct vpnhide_buf *b, const char *s)
{
	while (*s)
		vpnhide_putc(b, *s++);
}

static inline void vpnhide_put_dec(struct vpnhide_buf *b, unsigned long long v)
{
	char tmp[24];
	int n = 0;

	if (v == 0) {
		vpnhide_putc(b, '0');
		return;
	}
	while (v) {
		tmp[n++] = (char)('0' + (int)(v % 10ULL));
		v /= 10ULL;
	}
	while (n)
		vpnhide_putc(b, tmp[--n]);
}

/* Always lowercase out (§4.4: liberal-in / strict-out). */
static inline void vpnhide_put_hex(struct vpnhide_buf *b, unsigned long long v)
{
	static const char hexd[] = "0123456789abcdef";
	char tmp[16];
	int n = 0;

	vpnhide_puts(b, "0x");
	if (v == 0) {
		vpnhide_putc(b, '0');
		return;
	}
	while (v) {
		tmp[n++] = hexd[(int)(v & 0xfULL)];
		v >>= 4;
	}
	while (n)
		vpnhide_putc(b, tmp[--n]);
}

static inline void vpnhide_put_header(struct vpnhide_buf *b, const char *kind,
				      unsigned long version)
{
	vpnhide_puts(b, "vpnhide ");
	vpnhide_put_dec(b, version);
	vpnhide_putc(b, ' ');
	vpnhide_puts(b, kind);
	vpnhide_putc(b, '\n');
}

/*
 * Serialise a `stats` snapshot into buf[cap]. `e[0..n)` is grouped by uid
 * (consecutive entries with the same uid share one line); the producer emits
 * only non-zero cells. Returns the FULL length (may exceed cap — caller sizes
 * by its uid ceiling, or clamps with vpnhide_clamp_to_line for KPM out_msg).
 */
static inline unsigned long
vpnhide_format_stats(char *buf, unsigned long cap,
		     const struct vpnhide_stat_entry *e, int n)
{
	struct vpnhide_buf b;
	int idx = 0;

	b.p = buf;
	b.cap = cap;
	b.len = 0;
	vpnhide_put_header(&b, "stats", VPNHIDE_TELEMETRY_VERSION);
	while (idx < n) {
		unsigned int uid = e[idx].uid;

		vpnhide_put_hex(&b, uid);
		while (idx < n && e[idx].uid == uid) {
			vpnhide_putc(&b, ' ');
			vpnhide_put_hex(&b, e[idx].hook_id);
			vpnhide_putc(&b, ':');
			vpnhide_put_hex(&b, e[idx].count);
			idx++;
		}
		vpnhide_putc(&b, '\n');
	}
	return b.len;
}

/* Serialise a `status` snapshot into buf[cap]. Returns the FULL length. */
static inline unsigned long
vpnhide_format_status(char *buf, unsigned long cap,
		      const struct vpnhide_status *s)
{
	struct vpnhide_buf b;

	b.p = buf;
	b.cap = cap;
	b.len = 0;
	vpnhide_put_header(&b, "status", VPNHIDE_TELEMETRY_VERSION);
	vpnhide_puts(&b, "backend ");
	vpnhide_put_hex(&b, s->backend);
	vpnhide_putc(&b, '\n');
	vpnhide_puts(&b, "kver ");
	vpnhide_put_hex(&b, s->kver);
	vpnhide_putc(&b, '\n');
	vpnhide_puts(&b, "hooks ");
	vpnhide_put_hex(&b, s->hooks);
	vpnhide_putc(&b, '\n');
	vpnhide_puts(&b, "error ");
	vpnhide_put_hex(&b, s->error);
	vpnhide_putc(&b, '\n');
	return b.len;
}

/*
 * Clamp a fully-serialised snapshot (length `full_len`) to `outlen` bytes on a
 * LINE boundary, for the KPM single-buffer transport (§7.2). If the whole
 * snapshot fits, returns full_len unchanged — it ends in `\n`, signalling a
 * COMPLETE read. Otherwise returns the length of the largest run of complete
 * lines that fits, MINUS its trailing `\n`: the absent newline is the
 * truncation signal the reader checks, and no record is ever cut mid-line.
 */
static inline unsigned long vpnhide_clamp_to_line(const char *buf,
						  unsigned long full_len,
						  unsigned long outlen)
{
	unsigned long p;

	if (full_len <= outlen)
		return full_len;
	p = outlen;
	while (p > 0 && buf[p - 1] != '\n')
		p--;
	/* p sits just past the last newline within outlen (or 0). Drop that
	 * newline so the truncated read is recognisable by its missing \n. */
	return p ? p - 1 : 0;
}

/* ====================================================================== */
/*  kallsyms name matching (KPM symbol resolution).                       */
/* ====================================================================== */

static inline const char *vpnhide_skip_prefix(const char *value,
					      const char *prefix)
{
	int i;

	for (i = 0; prefix[i]; i++)
		if (value[i] != prefix[i])
			return 0;
	return value + i;
}

static inline int vpnhide_is_hex_digit(char c)
{
	return (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') ||
	       (c >= 'A' && c <= 'F');
}

/*
 * Does `suffix` (what follows the wanted function name in a kallsyms entry)
 * mark a compiler-renamed clone of that same function?
 *
 * A hook target can reach kallsyms under four observed forms:
 *   - GCC specialisation:   name.isra.N / name.constprop.N
 *   - Clang (Thin)LTO:      name.llvm.<decimal>
 *   - Clang CFI + full LTO: name$<hex>
 * The last one showed up on a MediaTek 4.14 vendor build, where sock_ioctl,
 * fib_route_seq_show and ipv6_route_seq_show all carried it (and their hooks
 * silently did not install) while every other target kept a plain or `.llvm.`
 * name.
 *
 * The digit/hex run must end the string. That is what rejects the CFI jump
 * table alias `name$<hex>.cfi_jt`, which sits right next to the real symbol:
 * hooking a trampoline would patch the wrong instructions. Cold fragments
 * (`name.cold`) and every other dotted form are rejected for the same reason —
 * they are not the complete function.
 */
static inline int vpnhide_symbol_suffix_is_clone(const char *suffix)
{
	const char *rest = vpnhide_skip_prefix(suffix, ".isra.");

	if (!rest)
		rest = vpnhide_skip_prefix(suffix, ".constprop.");
	if (!rest)
		rest = vpnhide_skip_prefix(suffix, ".llvm.");
	if (rest) {
		if (*rest < '0' || *rest > '9')
			return 0;
		do {
			rest++;
		} while (*rest >= '0' && *rest <= '9');
		return *rest == '\0';
	}
	rest = vpnhide_skip_prefix(suffix, "$");
	if (!rest || !vpnhide_is_hex_digit(*rest))
		return 0;
	do {
		rest++;
	} while (vpnhide_is_hex_digit(*rest));
	return *rest == '\0';
}

#endif /* VPNHIDE_SHARED_LOGIC_H */
