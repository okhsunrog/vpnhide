_2026-07-01_

## English

Zygisk blocks VPN interface-index probes (if_nametoindex / ioctl(SIOCGIFINDEX)) and intercepts /proc/net/{dev,udp,udp6} and /proc/thread-self and task path forms that could reveal a hidden VPN, and no longer reads out of bounds on netlink replies that use MSG_TRUNC.

## Русский

Zygisk блокирует запросы индекса VPN-интерфейса (if_nametoindex / ioctl(SIOCGIFINDEX)) и перехватывает /proc/net/{dev,udp,udp6} и формы путей /proc/thread-self и task, которые могли выдать скрытый VPN; устранён выход за границы буфера при netlink-чтении с флагом MSG_TRUNC.
