/* SPDX-License-Identifier: MIT */
/* Native consumer of the private Rust-owned diagnostic RAW wire. */
#ifndef VPNHIDE_DIAG_WIRE_H
#define VPNHIDE_DIAG_WIRE_H
#include "../generated/diagnostic_raw.h"
#include "vpnhide_diag_scope.h"

/* Called only with a kernel-owned 16-byte ifreq name at SIOCGIFNAME entry.
 * A normal name and a guessed token cannot register a different UID/TID.
 * Root activation remains separate; this never enables RAW by itself.
 */
static inline enum vpnhide_diag_result
vpnhide_diag_register_name(struct vpnhide_diag_scope *scope,
			   const struct vpnhide_diag_ops *ops,
			   const unsigned char name[16])
{
	const char *marker = VPNHIDE_DIAG_RAW_REGISTRATION_MARKER;
	unsigned long long token = 0;
	unsigned int i;
	if (!name || name[15])
		return VPNHIDE_DIAG_INVALID;
	for (i = 0; i < 7; i++)
		if (name[8 + i] != (unsigned char)marker[i])
			return VPNHIDE_DIAG_INVALID;
	for (i = 0; i < 8; i++)
		token |= (unsigned long long)name[i] << (i * 8);
	return vpnhide_diag_register(scope, ops, token);
}

enum vpnhide_diag_command_kind {
	VPNHIDE_DIAG_ARM,
	VPNHIDE_DIAG_ACTIVATE,
	VPNHIDE_DIAG_REVOKE,
};
struct vpnhide_diag_command {
	enum vpnhide_diag_command_kind kind;
	unsigned int tid, uid, duration_ms;
	unsigned long long nonce;
};
static inline int vpnhide_diag_word(const char **input, const char *end,
				    const char *word)
{
	const char *cursor = *input;

	while (*word) {
		if (cursor == end || *cursor++ != *word++)
			return 0;
	}
	*input = cursor;
	return 1;
}
static inline int vpnhide_diag_decimal(const char **input, const char *end,
				       unsigned int *output)
{
	const char *cursor = *input;
	unsigned int result = 0, digit;

	if (cursor == end || *cursor < '0' || *cursor > '9')
		return 0;
	while (cursor != end && *cursor >= '0' && *cursor <= '9') {
		digit = (unsigned int)(*cursor++ - '0');
		if (result > (~0u - digit) / 10)
			return 0;
		result = result * 10 + digit;
	}
	*output = result;
	*input = cursor;
	return 1;
}
static inline int vpnhide_diag_token(const char **input, const char *end,
				     unsigned long long *output)
{
	const char *cursor = *input;
	unsigned long long result = 0;
	unsigned int digit, i;

	for (i = 0; i < 16; i++) {
		if (cursor == end)
			return 0;
		if (*cursor >= '0' && *cursor <= '9')
			digit = (unsigned int)(*cursor - '0');
		else if (*cursor >= 'a' && *cursor <= 'f')
			digit = (unsigned int)(*cursor - 'a' + 10);
		else
			return 0;
		result = (result << 4) | digit;
		cursor++;
	}
	if (!result)
		return 0;
	*input = cursor;
	*output = result;
	return 1;
}
static inline int
vpnhide_diag_parse_command(const char *input, unsigned long bytes,
			   struct vpnhide_diag_command *command)
{
	const char *end;
	struct vpnhide_diag_command parsed = { 0 };

	if (!input || !command || !bytes ||
	    bytes > VPNHIDE_DIAG_RAW_COMMAND_BYTES || input[bytes - 1] != '\n')
		return 0;
	end = input + bytes - 1;
	if (!vpnhide_diag_word(&input, end, VPNHIDE_DIAG_RAW_PREFIX))
		return 0;
	if (vpnhide_diag_word(&input, end, "arm ")) {
		parsed.kind = VPNHIDE_DIAG_ARM;
		if (!vpnhide_diag_decimal(&input, end, &parsed.tid) ||
		    !vpnhide_diag_word(&input, end, " ") ||
		    !vpnhide_diag_decimal(&input, end, &parsed.uid) ||
		    !vpnhide_diag_word(&input, end, " ") ||
		    !vpnhide_diag_token(&input, end, &parsed.nonce) ||
		    !vpnhide_diag_word(&input, end, " ") ||
		    !vpnhide_diag_decimal(&input, end, &parsed.duration_ms) ||
		    !parsed.tid || parsed.tid > (~0u >> 1) || !parsed.uid ||
		    !parsed.duration_ms ||
		    parsed.duration_ms > VPNHIDE_DIAG_RAW_MAX_MS)
			return 0;
	} else {
		if (vpnhide_diag_word(&input, end, "activate "))
			parsed.kind = VPNHIDE_DIAG_ACTIVATE;
		else if (vpnhide_diag_word(&input, end, "revoke "))
			parsed.kind = VPNHIDE_DIAG_REVOKE;
		else
			return 0;
		if (!vpnhide_diag_token(&input, end, &parsed.nonce))
			return 0;
	}
	if (input != end)
		return 0;
	*command = parsed;
	return 1;
}
/* Privileged commands always check the CURRENT kernel credential. Possessing
 * an FD opened by root does not authorize another UID to arm/activate/revoke.
 */
static inline enum vpnhide_diag_result
vpnhide_diag_execute(struct vpnhide_diag_scope *scope,
		     const struct vpnhide_diag_ops *ops,
		     const struct vpnhide_diag_command *command)
{
	if (!vpnhide_diag_ops_valid(ops))
		return VPNHIDE_DIAG_UNAVAILABLE;
	if (ops->uid(ops->context) != 0)
		return VPNHIDE_DIAG_INVALID;
	switch (command->kind) {
	case VPNHIDE_DIAG_ARM:
		return vpnhide_diag_arm(scope, ops, command->tid, command->uid,
					command->nonce,
					command->duration_ms * 1000000ULL);
	case VPNHIDE_DIAG_ACTIVATE:
		return vpnhide_diag_activate(scope, ops, command->nonce);
	case VPNHIDE_DIAG_REVOKE:
		return vpnhide_diag_revoke(scope, ops, command->nonce);
	}
	return VPNHIDE_DIAG_INVALID;
}
static inline const char *vpnhide_diag_phase_name(enum vpnhide_diag_phase phase)
{
	switch (phase) {
	case VPNHIDE_DIAG_RELEASED:
		return "released";
	case VPNHIDE_DIAG_ARMED:
		return "armed";
	case VPNHIDE_DIAG_REGISTERED:
		return "registered";
	case VPNHIDE_DIAG_ACTIVE:
		return "active";
	}
	return "invalid";
}
#endif
