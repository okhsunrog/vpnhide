// SPDX-License-Identifier: MIT
/*
 * Minimal 32-bit (compat) SO_BINDTODEVICE probe for the QEMU harness.
 *
 * The full bind-probe.c is arm64-only (hand-rolled raw syscalls). This tiny
 * companion exists to exercise ONE thing the 64-bit probe cannot: the 32-bit
 * compat setsockopt path. On pre-5.9 kernels that path is a separate entry
 * (compat_sock_setsockopt), so a bind hook patched only at the 64-bit
 * __sys_setsockopt would leak here. Built for armv7 and run under CONFIG_COMPAT,
 * it goes through the kernel's compat setsockopt handler via ordinary libc — the
 * built-in backend has no userspace hook, so libc is the honest path a real
 * 32-bit app would take.
 *
 * Runs as the actor UID (10000, the harness's target uid) so the hook fires,
 * then reports errno + the read-back bind state in the same key format the
 * 64-bit probe uses, so init.sh parses it the same way:
 *   BIND_NAME_RAW_ERRNO=<errno of setsockopt, 0 on success>
 *   BIND_NAME_RAW_STATE=<0 unbound | 1 bound-to-name | 2 bound-elsewhere | <0 err>
 */
#include <errno.h>
#include <grp.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

#ifndef SO_BINDTODEVICE
#define SO_BINDTODEVICE 25
#endif
#define ACTOR_UID 10000
#define AID_INET 3003

int main(int argc, char **argv)
{
	const char *name = argc > 1 ? argv[1] : "vpn0";
	gid_t groups[] = { AID_INET };
	int fd, err = 0, state;
	char buf[64] = { 0 };
	socklen_t len = sizeof(buf);

	if (setgroups(1, groups) != 0 || setgid(ACTOR_UID) != 0 ||
	    setuid(ACTOR_UID) != 0) {
		printf("BIND_NAME_RAW_ERRNO=-1\nBIND_NAME_RAW_STATE=-1\n");
		return 0;
	}

	fd = socket(AF_INET, SOCK_STREAM, 0);
	if (fd < 0) {
		printf("BIND_NAME_RAW_ERRNO=%d\nBIND_NAME_RAW_STATE=-1\n", errno);
		return 0;
	}

	if (setsockopt(fd, SOL_SOCKET, SO_BINDTODEVICE, name, strlen(name)) != 0)
		err = errno;

	if (getsockopt(fd, SOL_SOCKET, SO_BINDTODEVICE, buf, &len) != 0)
		state = -errno;
	else if (len == 0 || buf[0] == '\0')
		state = 0;
	else if (strcmp(buf, name) == 0)
		state = 1;
	else
		state = 2;

	printf("BIND_NAME_RAW_ERRNO=%d\nBIND_NAME_RAW_STATE=%d\n", err, state);
	close(fd);
	return 0;
}
