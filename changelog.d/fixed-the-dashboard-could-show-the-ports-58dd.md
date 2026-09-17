_2026-09-17_

## English

The Dashboard could show the Ports module as inactive, with a warning, after a root snapshot raced another iptables user (netd on a network change, the VPN client). The probe now waits for the xtables lock, and a probe that still could not run shows the module as not verified instead of inactive.

## Русский

Главный экран мог показать модуль Ports как неактивный с предупреждением, если root-снимок совпал с чужим вызовом iptables (netd при смене сети, VPN-клиент). Проба теперь ждёт xtables-lock, а если всё же не смогла выполниться, модуль показывается как «не проверен», а не как неактивный.
