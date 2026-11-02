// SPDX-License-Identifier: MIT
/* Offline QEMU oracle only. The host binds these exact final ELF bytes by hash;
 * this program is never a production candidate-selection or retry loop. */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

int main(int argc, char **argv)
{
	struct stat metadata;
	char *end;
	long expected, result;
	int fd, error;

	if (argc != 4)
		return 64;
	errno = 0;
	expected = strtol(argv[3], &end, 10);
	if (errno || !*argv[3] || *end || expected < 0 || expected > 4095)
		return 64;
	fd = open(argv[1], O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
	if (fd < 0)
		return 65;
	if (fstat(fd, &metadata) || !S_ISREG(metadata.st_mode)) {
		close(fd);
		return 65;
	}
	errno = 0;
	result = syscall(SYS_finit_module, fd, argv[2], 0);
	error = errno;
	close(fd);
	printf("SIGNATURE_FILE=%s RETURN=%ld ERRNO=%d EXPECTED=%ld\n", argv[1],
	       result, error, expected);
	return expected == 0 ? result != 0 :
			       !(result == -1 && error == expected);
}
