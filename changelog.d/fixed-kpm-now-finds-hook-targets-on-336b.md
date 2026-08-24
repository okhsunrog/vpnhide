_2026-08-22_

## English

KPM now finds hook targets on kernels that rename them (Clang CFI with full LTO, seen on MediaTek 4.14). Three hooks silently failed to install there, which left ioctl(SIOCGIFCONF) enumerating the VPN interface for target apps.

## Русский

KPM теперь находит функции ядра, которые в некоторых сборках названы иначе (Clang CFI с full LTO, замечено на MediaTek 4.14). Там молча не вставали три хука, из-за чего ioctl(SIOCGIFCONF) продолжал показывать VPN-интерфейс приложениям-целям.
