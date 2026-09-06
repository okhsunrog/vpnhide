// SPDX-License-Identifier: MIT
/*
 * By-name SIOCGIFHWADDR / SIOCGIFADDR probe for the QEMU harness.
 *
 * These two get-by-name ioctls bypass dev_ifsioc_locked — SIOCGIFHWADDR via
 * dev_get_mac_address() (5.4+) and SIOCGIFADDR via devinet_ioctl() — so a
 * caller that guesses a hidden interface's name could read its MAC / IPv4
 * address even though enumeration is concealed. Ordinary libc: the built-in
 * backend has no userspace hook, so this is the honest path a real app takes.
 *
 * Runs as the actor UID (10000, the harness's target uid) so the hook fires,
 * and reports each ioctl's errno:
 *   HWADDR_ERRNO=<errno, 0 on success>
 *   ADDR_ERRNO=<errno, 0 on success>
 * A hidden interface must answer -ENODEV (19) for both, exactly as a
 * genuinely-absent name would; a visible one answers 0 (or another non-ENODEV
 * errno, e.g. EADDRNOTAVAIL for an interface with no IPv4).
 */
#include <errno.h>
#include <grp.h>
#include <net/if.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <unistd.h>

#include <linux/sockios.h>

#define ACTOR_UID 10000
#define AID_INET 3003

int main(int argc, char **argv)
{
	const char *name = argc > 1 ? argv[1] : "vpn0";
	gid_t groups[] = { AID_INET };
	struct ifreq ifr;
	int fd, hw = 0, ad = 0;

	if (setgroups(1, groups) != 0 || setgid(ACTOR_UID) != 0 ||
	    setuid(ACTOR_UID) != 0) {
		printf("HWADDR_ERRNO=-1\nADDR_ERRNO=-1\n");
		return 0;
	}

	fd = socket(AF_INET, SOCK_DGRAM, 0);
	if (fd < 0) {
		printf("HWADDR_ERRNO=-1\nADDR_ERRNO=-1\n");
		return 0;
	}

	memset(&ifr, 0, sizeof(ifr));
	strncpy(ifr.ifr_name, name, IFNAMSIZ - 1);
	if (ioctl(fd, SIOCGIFHWADDR, &ifr) != 0)
		hw = errno;

	memset(&ifr, 0, sizeof(ifr));
	strncpy(ifr.ifr_name, name, IFNAMSIZ - 1);
	if (ioctl(fd, SIOCGIFADDR, &ifr) != 0)
		ad = errno;

	printf("HWADDR_ERRNO=%d\nADDR_ERRNO=%d\n", hw, ad);
	close(fd);
	return 0;
}
