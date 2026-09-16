/*
 * Host harness for the control/stats protocol golden vectors
 * (protocol_vectors.tsv) — the C side of the cross-language parity check
 * (docs/protocol.md §8 Layer 1). Drives the freestanding parser/serialiser in
 * shared/vpnhide_logic.h against the same vectors the Rust/Kotlin ports will.
 *
 * Build: gcc -O2 -Wall -Wextra -Werror -I.. -o test_protocol test_protocol.c
 * Run:   ./test_protocol [protocol_vectors.tsv]   (exit 0 on success)
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "shared/vpnhide_logic.h"

#define MAX_TARGETS 256
/* Config capacity must equal vpnhide_protocol::MAX_TARGET_UIDS: the shared
 * vectors pin over-ceiling payloads as REJECT, so C and Rust have to be
 * rejecting at the same count. */
#define CFG_MAX_TARGETS 160
#define MAX_FIELDS 8

static int failures;
static int checks;

/* Decode \n \t \\ \xNN escapes from `in` into `out` (cap bytes); returns the
 * decoded length. The vectors file keeps payloads on one physical line. */
static unsigned long decode(const char *in, char *out, unsigned long cap)
{
	unsigned long n = 0;

	while (*in && n < cap) {
		if (in[0] == '\\' && in[1]) {
			char c = in[1];

			if (c == 'n') {
				out[n++] = '\n';
				in += 2;
			} else if (c == 't') {
				out[n++] = '\t';
				in += 2;
			} else if (c == 'r') {
				out[n++] = '\r';
				in += 2;
			} else if (c == '\\') {
				out[n++] = '\\';
				in += 2;
			} else if (c == 'x' && in[2] && in[3]) {
				char hx[3] = { in[2], in[3], 0 };

				out[n++] = (char)strtoul(hx, NULL, 16);
				in += 4;
			} else {
				out[n++] = *in++;
			}
		} else {
			out[n++] = *in++;
		}
	}
	out[n < cap ? n : cap - 1] = '\0'; /* strtok/strcmp paths need this */
	return n;
}

/* Split `line` in place on '|' into up to MAX_FIELDS raw (un-decoded) fields. */
static int split_pipe(char *line, char *fields[])
{
	int nf = 0;
	char *p = line;

	fields[nf++] = p;
	while (*p && nf < MAX_FIELDS) {
		if (*p == '|') {
			*p = '\0';
			fields[nf++] = p + 1;
		}
		p++;
	}
	return nf;
}

static void fail(const char *what, const char *got, const char *want)
{
	fprintf(stderr, "FAIL %s\n  got : <%s>\n  want: <%s>\n", what, got,
		want);
	failures++;
}

/* ---- cfg: parse_config ------------------------------------------------- */

static void run_cfg(const char *raw_in, const char *expect)
{
	char in[2048];
	unsigned long len = decode(raw_in, in, sizeof(in));
	struct vpnhide_target out[CFG_MAX_TARGETS];
	int debug = -1;
	unsigned int default_mask = 0;
	int n = vpnhide_parse_config(in, len, out, CFG_MAX_TARGETS, &debug,
				     &default_mask);

	checks++;
	if (strcmp(expect, "REJECT") == 0) {
		if (n != -1)
			fail("cfg expected REJECT", "accepted", "REJECT");
		return;
	}
	if (n < 0) {
		fail("cfg unexpectedly rejected", "REJECT", expect);
		return;
	}

	/* Build "debug=<d>;uid:hm;..." from the parse result and compare. */
	char got[2048];
	int pos = snprintf(got, sizeof(got), "debug=%d;def=0x%x", debug,
			   default_mask);
	for (int i = 0; i < n; i++)
		pos += snprintf(got + pos, sizeof(got) - (size_t)pos,
				";0x%x:0x%x", out[i].uid, out[i].hookmask);
	if (strcmp(got, expect) != 0)
		fail("cfg parse mismatch", got, expect);
}

/* ---- kind: peek_kind --------------------------------------------------- */

static void run_kind(const char *raw_in, const char *expect)
{
	char in[1024];
	unsigned long len = decode(raw_in, in, sizeof(in));
	enum vpnhide_kind k = vpnhide_peek_kind(in, len);
	const char *got = k == VPNHIDE_KIND_CONFIG ? "CONFIG" :
			  k == VPNHIDE_KIND_STATS  ? "STATS" :
			  k == VPNHIDE_KIND_STATUS ? "STATUS" :
						     "INVALID";

	checks++;
	if (strcmp(got, expect) != 0)
		fail("kind mismatch", got, expect);
}

/* ---- stats: format_stats ----------------------------------------------- */

static void run_stats(const char *raw_entries, const char *raw_expect)
{
	struct vpnhide_stat_entry e[MAX_TARGETS];
	int n = 0;
	char entries[2048];
	char expect[2048];
	char out[4096];

	decode(raw_entries, entries, sizeof(entries));
	decode(raw_expect, expect, sizeof(expect));

	/* entries: "uid,hid,cnt;uid,hid,cnt;..." */
	char *save1 = NULL;
	for (char *grp = strtok_r(entries, ";", &save1); grp && n < MAX_TARGETS;
	     grp = strtok_r(NULL, ";", &save1)) {
		char *save2 = NULL;
		char *a = strtok_r(grp, ",", &save2);
		char *b = strtok_r(NULL, ",", &save2);
		char *c = strtok_r(NULL, ",", &save2);

		if (!a || !b || !c)
			continue;
		e[n].uid = (unsigned int)strtoull(a, NULL, 0);
		e[n].hook_id = (unsigned int)strtoull(b, NULL, 0);
		e[n].count = strtoull(c, NULL, 0);
		n++;
	}

	unsigned long full = vpnhide_format_stats(out, sizeof(out), e, n);

	checks++;
	out[full < sizeof(out) ? full : sizeof(out) - 1] = '\0';
	if (strcmp(out, expect) != 0)
		fail("stats format mismatch", out, expect);
}

/* ---- status: format_status --------------------------------------------- */

static void run_status(const char *raw_fields, const char *raw_expect)
{
	char fields[256];
	char expect[1024];
	char out[1024];
	struct vpnhide_status s;

	decode(raw_fields, fields, sizeof(fields));
	decode(raw_expect, expect, sizeof(expect));

	char *save = NULL;
	char *a = strtok_r(fields, ",", &save);
	char *b = strtok_r(NULL, ",", &save);
	char *c = strtok_r(NULL, ",", &save);
	char *d = strtok_r(NULL, ",", &save);

	if (!a || !b || !c || !d) {
		fail("status vector malformed", raw_fields, "4 fields");
		return;
	}
	s.backend = (unsigned int)strtoull(a, NULL, 0);
	s.kver = (unsigned int)strtoull(b, NULL, 0);
	s.hooks = (unsigned int)strtoull(c, NULL, 0);
	s.error = (unsigned int)strtoull(d, NULL, 0);

	unsigned long full = vpnhide_format_status(out, sizeof(out), &s);

	checks++;
	out[full < sizeof(out) ? full : sizeof(out) - 1] = '\0';
	if (strcmp(out, expect) != 0)
		fail("status format mismatch", out, expect);
}

/* ---- clamp: clamp_to_line ---------------------------------------------- */

static void run_clamp(const char *raw_full, const char *raw_outlen,
		      const char *raw_expect)
{
	char full[1024];
	char expect[1024];
	unsigned long flen = decode(raw_full, full, sizeof(full));
	unsigned long elen = decode(raw_expect, expect, sizeof(expect));
	unsigned long outlen = strtoul(raw_outlen, NULL, 10);
	unsigned long got = vpnhide_clamp_to_line(full, flen, outlen);

	checks++;
	if (got != elen || memcmp(full, expect, elen) != 0) {
		char g[1024];

		memcpy(g, full, got);
		g[got] = '\0';
		fail("clamp mismatch", g, expect);
	}
}

/* KPM formats into a fixed buffer while format_stats returns the larger,
 * intended length. The clamp must still treat an exactly-full output buffer as
 * truncated and remove its final newline; replacing full_len with sizeof(buf)
 * used to make this case look complete. */
static void run_fixed_buffer_overflow_clamp(void)
{
	char written[8] = { 'a', 'a', 'a', '\n', 'b', 'b', 'b', '\n' };
	unsigned long got = vpnhide_clamp_to_line(written, 12, sizeof(written));

	checks++;
	if (got != 7 || memcmp(written, "aaa\nbbb", 7) != 0)
		fail("fixed-buffer overflow clamp", "not line-clamped",
		     "aaa\\nbbb");
}

static void run_stats_after_parse(void)
{
	static const struct {
		const char *wire;
		int valid;
		unsigned int after;
	} cases[] = {
		{ "vpnhide 1 stats", 1, 0 },
		{ "vpnhide 1 stats\nafter 0x27ff\n", 1, 0x27ff },
		{ "vpnhide 1 stats\nfuture value\nafter 0XFFFFFFFF", 1,
		  0xffffffffu },
		{ "vpnhide 1 status\nafter 0x1", 0, 0 },
		{ "vpnhide 1 stats\nafter 27ff", 0, 0 },
		{ "vpnhide 1 stats\nafter 0x1 extra", 0, 0 },
		{ "vpnhide 1 stats\nafter 0x1\nafter 0x2", 0, 0 },
	};
	unsigned int i;

	for (i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
		unsigned int after = 123;
		int valid = vpnhide_parse_stats_after(
			cases[i].wire, strlen(cases[i].wire), &after);

		checks++;
		if (valid != cases[i].valid ||
		    (valid && after != cases[i].after)) {
			char got[64], want[64];

			snprintf(got, sizeof(got), "%d,0x%x", valid, after);
			snprintf(want, sizeof(want), "%d,0x%x", cases[i].valid,
				 cases[i].after);
			fail("stats after parse", got, want);
		}
	}
}

int main(int argc, char **argv)
{
	const char *path = argc > 1 ? argv[1] : "protocol_vectors.tsv";
	FILE *f = fopen(path, "r");
	char *line = NULL;
	size_t cap = 0;
	ssize_t got;

	if (!f) {
		fprintf(stderr, "cannot open vectors file: %s\n", path);
		return 2;
	}

	while ((got = getline(&line, &cap, f)) != -1) {
		char *fields[MAX_FIELDS];
		int nf;

		if (got && line[got - 1] == '\n')
			line[got - 1] = '\0';
		if (line[0] == '\0' || line[0] == '#')
			continue;

		nf = split_pipe(line, fields);
		if (nf < 2)
			continue;

		if (strcmp(fields[0], "cfg") == 0 && nf >= 3)
			run_cfg(fields[1], fields[2]);
		else if (strcmp(fields[0], "kind") == 0 && nf >= 3)
			run_kind(fields[1], fields[2]);
		else if (strcmp(fields[0], "stats") == 0 && nf >= 3)
			run_stats(fields[1], fields[2]);
		/* Kernel C emits status; Rust/Kotlin consume these read vectors. */
		else if (strcmp(fields[0], "status_fields") == 0)
			continue;
		else if (strcmp(fields[0], "status") == 0 && nf >= 3)
			run_status(fields[1], fields[2]);
		else if (strcmp(fields[0], "clamp") == 0 && nf >= 4)
			run_clamp(fields[1], fields[2], fields[3]);
		else
			fprintf(stderr, "WARN unrecognised vector: %s\n",
				fields[0]);
	}

	free(line);
	fclose(f);
	run_fixed_buffer_overflow_clamp();
	run_stats_after_parse();

	if (failures) {
		fprintf(stderr, "%d/%d protocol vector(s) failed\n", failures,
			checks);
		return 1;
	}
	printf("all %d protocol vectors passed\n", checks);
	return 0;
}
