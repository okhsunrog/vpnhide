// SPDX-License-Identifier: MIT
/* QEMU-only syscall oracle. The host harness must first verify protocol 1 in
 * this exact ELF, and bind the initramfs copy by SHA-256. Never probe an old ko
 * by passing an unknown parameter: Linux may ignore it and execute full init.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

int main(int argc, char **argv)
{
	struct stat metadata;
	int fd, error;
	long result;

	if (argc != 2)
		return 64;
	fd = open(argv[1], O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
	if (fd < 0)
		return 65;
	if (fstat(fd, &metadata) || !S_ISREG(metadata.st_mode)) {
		close(fd);
		return 65;
	}
	errno = 0;
	result = syscall(SYS_finit_module, fd, "preflight_only=1", 0);
	error = errno;
	close(fd);
	printf("PREFLIGHT_RETURN=%ld PREFLIGHT_ERRNO=%d\n", result, error);
	return result == -1 && error == EUCLEAN ? 0 : 1;
}
