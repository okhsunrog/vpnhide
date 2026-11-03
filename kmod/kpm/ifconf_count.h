/* SPDX-License-Identifier: MIT */
/* Read-only NULL-query projection. Both production and host behavioral tests
 * compile this implementation. All kernel reads are no-fault and traversal is
 * bounded under RTNL. Only a complete count matching the kernel's own output
 * can authorize changing the user-visible length.
 */
#ifndef VPNHIDE_KPM_IFCONF_COUNT_H
#define VPNHIDE_KPM_IFCONF_COUNT_H

#define VPNHIDE_IFCONF_MAX_OBJECTS 4096
static const struct vpnhide_ifconf_offsets *ifconf_offsets;
static unsigned int ifconf_offset_count;
static void *ifconf_sock_from_file;
static void (*ifconf_rtnl_lock)(void);
static void (*ifconf_rtnl_unlock)(void);
static long (*ifconf_kernel_read)(void *, const void *, unsigned long);

static void resolve_ifconf_count_symbols(void)
{
	ifconf_offsets = vpnhide_select_ifconf_offsets((unsigned int)kver,
						       &ifconf_offset_count);
	ifconf_sock_from_file = (void *)kallsyms_lookup_name("sock_from_file");
	ifconf_rtnl_lock = (void *)kallsyms_lookup_name("rtnl_lock");
	ifconf_rtnl_unlock = (void *)kallsyms_lookup_name("rtnl_unlock");
	ifconf_kernel_read =
		(void *)kallsyms_lookup_name("copy_from_kernel_nofault");
	if (!ifconf_kernel_read)
		ifconf_kernel_read =
			(void *)kallsyms_lookup_name("probe_kernel_read");
	if (!ifconf_offsets || !ifconf_sock_from_file || !ifconf_rtnl_lock ||
	    !ifconf_rtnl_unlock || !ifconf_kernel_read)
		logki(MODNAME
		      ": NULL ifconf count remains unverified: missing qualified layout/helper\n");
}

static int ifconf_read_ptr(const void *at, void **value)
{
	return ptr_is_kernel(at) &&
	       !ifconf_kernel_read(value, at, sizeof(*value));
}

static int ifconf_walk(const struct vpnhide_ifconf_offsets *o, void *net,
		       int expected, int *hidden)
{
	char *head = (char *)net + o->net_dev_base_head;
	void *node;
	int objects = 0, count = 0;

	*hidden = 0;
	if (!ifconf_read_ptr(head, &node))
		return 0;
	while (node != (void *)head) {
		void *next, *prev, *in_dev, *dev_check, *ifa;
		char *dev;

		if (!ptr_is_kernel(node))
			return 0;
		dev = (char *)node - o->netdev_dev_list;

		if (++objects > VPNHIDE_IFCONF_MAX_OBJECTS ||
		    !ifconf_read_ptr(node, &next) ||
		    !ifconf_read_ptr((char *)next + sizeof(void *), &prev) ||
		    prev != node ||
		    !ifconf_read_ptr(dev + o->netdev_ip_ptr, &in_dev))
			return 0;
		if (in_dev) {
			if (!ifconf_read_ptr((char *)in_dev +
						     off->in_device_dev,
					     &dev_check) ||
			    dev_check != dev ||
			    !ifconf_read_ptr((char *)in_dev +
						     o->in_device_ifa_list,
					     &ifa))
				return 0;
			while (ifa) {
				char label[VPNHIDE_IFNAMSIZ];
				void *owner;
				int terminated = 0;

				if (++objects > VPNHIDE_IFCONF_MAX_OBJECTS ||
				    !ifconf_read_ptr(
					    (char *)ifa +
						    off->in_ifaddr_ifa_dev,
					    &owner) ||
				    owner != in_dev ||
				    ifconf_kernel_read(
					    label,
					    (char *)ifa + o->in_ifaddr_label,
					    sizeof(label)) ||
				    !ifconf_read_ptr((char *)ifa +
							     o->in_ifaddr_next,
						     &ifa))
					return 0;
				for (int i = 0; i < VPNHIDE_IFNAMSIZ; i++)
					if (!label[i]) {
						terminated = 1;
						break;
					}
				if (!terminated || ++count > expected)
					return 0;
				if (iface_is_vpn(label))
					(*hidden)++;
			}
		}
		node = next;
	}
	return count == expected;
}

static int ifconf_project_length(void *file, int original, int *projected)
{
	void *socket, *sk, *net;
	int error = 0, hidden = 0, matched = 0;

	if (!ifconf_offsets || !ifconf_sock_from_file || !ifconf_rtnl_lock ||
	    !ifconf_rtnl_unlock || !ifconf_kernel_read || original <= 0 ||
	    original % VPNHIDE_IFREQ_SZ || !ptr_is_kernel(file))
		return 0;
	if (ifconf_offsets->sock_from_file_args == 1)
		socket = ((void *(*)(void *))ifconf_sock_from_file)(file);
	else if (ifconf_offsets->sock_from_file_args == 2)
		socket = ((void *(*)(void *, int *))ifconf_sock_from_file)(
			file, &error);
	else
		return 0;
	if (error || !ptr_is_kernel(socket) ||
	    !ifconf_read_ptr((char *)socket + off->socket_sk, &sk) ||
	    !ptr_is_kernel(sk) ||
	    !ifconf_read_ptr((char *)sk + off->sock_net, &net) ||
	    !ptr_is_kernel(net))
		return 0;
	ifconf_rtnl_lock();
	for (unsigned int i = 0; i < ifconf_offset_count; i++) {
		int candidate_hidden;
		if (ifconf_walk(&ifconf_offsets[i], net,
				original / VPNHIDE_IFREQ_SZ,
				&candidate_hidden)) {
			matched++;
			hidden = candidate_hidden;
		}
	}
	ifconf_rtnl_unlock();
	if (matched != 1 || !hidden)
		return 0;
	*projected = original - hidden * VPNHIDE_IFREQ_SZ;
	return 1;
}

#endif
