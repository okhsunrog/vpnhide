/* SPDX-License-Identifier: MIT */
#ifndef VPNHIDE_INIT_CONTRACT_H
#define VPNHIDE_INIT_CONTRACT_H
#ifdef __KERNEL__
#include <linux/errno.h>
#else
#include <errno.h>
#endif

#define VPNHIDE_INIT_CONTRACT_VERSION "1"

/* Keep loader-signature errno values outside every possible driver-init return.
 * This does not make late initialization failures safe: fallible work must still
 * precede externally reachable redirection hooks. See vpnhide_init_impl().
 */
static inline int vpnhide_normalize_init_result(int result)
{
	switch (result) {
	case 0:
	case -EUCLEAN:
	case -ENOENT:
	case -ENOMEM:
		return result;
	default:
		return -EIO;
	}
}
#endif
