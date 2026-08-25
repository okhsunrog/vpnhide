# Per-user package inventory, shared by the root snapshot and the debug export.
#
# Enumerates users first, then queries each one explicitly. The all-users
# shortcut is incomplete on some OEM ROMs, and tests assert its flag never
# appears in the generated command — do not name it here either.
#
# Inputs (exported by the Kotlin caller, see ShellScripts.kt):
#   VPNHIDE_SECTION_BEGIN / VPNHIDE_SECTION_END  section framing prefixes
#   VPNHIDE_PM_USERS_STATUS / VPNHIDE_PM_USER_BEGIN / VPNHIDE_PM_USER_END
#   VPNHIDE_PM_STDERR_TO_STDOUT   1 = fold pm's stderr into the section (bundle),
#                                 0 = drop it (normal snapshot)

pm_run() {
	if [ "$VPNHIDE_PM_STDERR_TO_STDOUT" = 1 ]; then
		"$@" 2>&1
	else
		"$@" 2>/dev/null
	fi
}

vpnhide_package_inventory() {
	PM_USERS_VERBOSE="$(pm_run pm list users -v)"
	PM_USERS_VERBOSE_STATUS=$?
	PM_USERS_PLAIN="$(pm_run pm list users)"
	PM_USERS_PLAIN_STATUS=$?
	echo "${VPNHIDE_SECTION_BEGIN}pm_users"
	echo "${VPNHIDE_PM_USERS_STATUS}verbose:$PM_USERS_VERBOSE_STATUS"
	echo "${VPNHIDE_PM_USERS_STATUS}plain:$PM_USERS_PLAIN_STATUS"
	[ -n "$PM_USERS_VERBOSE" ] && printf '%s\n' "$PM_USERS_VERBOSE"
	[ -n "$PM_USERS_PLAIN" ] && printf '%s\n' "$PM_USERS_PLAIN"
	echo "${VPNHIDE_SECTION_END}pm_users"

	PM_PLAIN_USER_IDS="$(printf '%s\n' "$PM_USERS_PLAIN" | sed -n 's/.*UserInfo{\([0-9][0-9]*\):.*/\1/p')"
	PM_VERBOSE_USER_IDS="$(printf '%s\n' "$PM_USERS_VERBOSE" | sed -n 's/^[[:space:]]*[0-9][0-9]*:[[:space:]]*id=\([0-9][0-9]*\),.*/\1/p')"
	PM_USER_IDS="$(printf '%s\n%s\n' "$PM_PLAIN_USER_IDS" "$PM_VERBOSE_USER_IDS" | sed '/^$/d' | sort -n -u)"

	echo "${VPNHIDE_SECTION_BEGIN}pm_packages"
	if [ -z "$PM_USER_IDS" ]; then
		PM_USER_IDS=0
	fi
	for PM_USER_ID in $PM_USER_IDS; do
		echo "$VPNHIDE_PM_USER_BEGIN$PM_USER_ID"
		# Stream pm's stdout straight into the section instead of staging it in a
		# shell variable and re-emitting it. A device with a very large app list
		# (bloatware-heavy MIUI/HyperOS) produces a package list bigger than the
		# kernel single-argument limit (MAX_ARG_STRLEN, ~128 KiB); passing it as one
		# argv word to printf/echo fails with "Argument list too long" and emits
		# nothing, so an exit-0 scan looked like an empty inventory. pm is the last
		# command in the section, so $? below is pm's own exit status.
		pm_run pm list packages -U -f --user "$PM_USER_ID"
		PM_USER_STATUS=$?
		echo "$VPNHIDE_PM_USER_END$PM_USER_ID:$PM_USER_STATUS"
	done
	echo "${VPNHIDE_SECTION_END}pm_packages"
}
