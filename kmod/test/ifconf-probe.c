/*
 * SIOCGIFCONF probe for the kmod harness — exercises the legacy ioctl
 * enumeration path, specifically the two-step "size query then fill" sequence
 * the `ip addr`/`ifconfig` shell vectors don't isolate.
 *
 * The classic caller first issues SIOCGIFCONF with ifc_req == NULL to learn how
 * big a buffer it needs (the kernel returns ifc_len = N * sizeof(struct ifreq)
 * without copying anything), then issues a sized call to fill it. If the kmod
 * filters the fill but leaves the size query unfiltered, the two disagree by
 * one interface for a hidden VPN — a detectable tell. This probe reports both
 * so the harness can assert they stay consistent.
 *
 * Build (static, runs on the Alpine VM regardless of its libc):
 *   <ndk>/aarch64-linux-android35-clang -static -O2 -o ifconf ifconf-probe.c
 * Output:
 *   IFCONF_SIZE=<n>      ifreqs reported by the NULL size-query
 *   IFCONF_FILL=<n>      ifreqs returned by a real fill
 *   IFCONF_FILL_VPN=<n>  of the fill entries, how many name "vpn0"
 */
#include <net/if.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <unistd.h>

int main(void)
{
	struct ifconf ifc;
	struct ifreq buf[64];
	int fd, i, size_n, fill_n, vpn = 0;

	fd = socket(AF_INET, SOCK_DGRAM, 0);
	if (fd < 0) {
		printf("IFCONF_SIZE=-1\nIFCONF_FILL=-1\nIFCONF_FILL_VPN=-1\n");
		return 2;
	}

	/* Step 1: size query — ifc_req == NULL, kernel returns only ifc_len. */
	ifc.ifc_len = 0;
	ifc.ifc_buf = NULL;
	if (ioctl(fd, SIOCGIFCONF, &ifc) < 0) {
		printf("IFCONF_SIZE=-1\nIFCONF_FILL=-1\nIFCONF_FILL_VPN=-1\n");
		close(fd);
		return 2;
	}
	size_n = ifc.ifc_len / (int)sizeof(struct ifreq);

	/* Step 2: real fill into a fixed buffer. */
	ifc.ifc_len = (int)sizeof(buf);
	ifc.ifc_req = buf;
	if (ioctl(fd, SIOCGIFCONF, &ifc) < 0) {
		printf("IFCONF_SIZE=%d\nIFCONF_FILL=-1\nIFCONF_FILL_VPN=-1\n",
		       size_n);
		close(fd);
		return 2;
	}
	fill_n = ifc.ifc_len / (int)sizeof(struct ifreq);
	for (i = 0; i < fill_n; i++)
		if (strcmp(buf[i].ifr_name, "vpn0") == 0)
			vpn++;

	close(fd);
	printf("IFCONF_SIZE=%d\nIFCONF_FILL=%d\nIFCONF_FILL_VPN=%d\n", size_n,
	       fill_n, vpn);
	return 0;
}
