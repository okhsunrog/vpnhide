/* SPDX-License-Identifier: MIT */
#include <assert.h>
#include <limits.h>
#include "shared/vpnhide_init_contract.h"

int main(void)
{
	int result;

	for (result = -4095; result <= 4095; result++) {
		int expected = result == 0 || result == -EUCLEAN ||
					       result == -ENOENT ||
					       result == -ENOMEM ?
				       result :
				       -EIO;

		assert(vpnhide_normalize_init_result(result) == expected);
		assert(vpnhide_normalize_init_result(result) != -ENOKEY);
		assert(vpnhide_normalize_init_result(result) != -EKEYREJECTED);
	}
	assert(vpnhide_normalize_init_result(INT_MIN) == -EIO);
	assert(vpnhide_normalize_init_result(INT_MAX) == -EIO);
	return 0;
}
