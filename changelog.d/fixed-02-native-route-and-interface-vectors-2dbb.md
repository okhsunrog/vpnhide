_2026-07-01_

## English

Target apps can no longer detect a hidden VPN by enumerating interfaces or reading routes directly from the kernel. VPN routes — and the physical host-route hints that expose them — are stripped from RTM_GETROUTE netlink dumps (including the FORTIFY'd recvfrom/__recvfrom_chk path) and from /proc/net/ipv6_route, the SIOCGIFCONF buffer-size count trick (ifc_req == NULL) is closed, and tunnels renamed to the kernel-default `if<N>` pattern (issue #86) — along with utun/l2tp/gre and renamed *vpn* interfaces — are recognized and hidden consistently across the kmod, native, and Java backends. Hiding interfaces from RTM_GETLINK dumps also no longer hangs under Permissive SELinux.

## Русский

Целевые приложения больше не могут обнаружить скрытый VPN, перечисляя интерфейсы или читая маршруты напрямую из ядра. VPN-маршруты и физические host-route признаки убираются из netlink-дампов RTM_GETROUTE (включая FORTIFY-обёртки recvfrom/__recvfrom_chk) и из /proc/net/ipv6_route, закрыт трюк с размером буфера SIOCGIFCONF (ifc_req == NULL), а туннели, переименованные в дефолтный для ядра паттерн `if<N>` (issue #86), — как и интерфейсы utun*, l2tp*, gre* и переименованные *vpn*, — распознаются и скрываются одинаково во всех бэкендах: kmod, нативном и Java. Кроме того, дампы RTM_GETLINK со скрытыми интерфейсами больше не зависают при Permissive SELinux.
