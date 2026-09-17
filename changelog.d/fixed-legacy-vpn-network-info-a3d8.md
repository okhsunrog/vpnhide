_2026-09-16_

## English

Legacy VPN network queries now preserve the disconnected VPN type instead of returning null or renaming it to Wi-Fi, avoiding false detection by apps such as older Ruru releases. Diagnostics now check the returned legacy network type and its enumeration as well as connection state; a null reply is reported as inconclusive instead of a successful check.

## Русский

Legacy-запросы VPN-сети теперь сохраняют отключённый тип VPN вместо null или подмены на Wi-Fi, устраняя ложное обнаружение в приложениях вроде старых версий Ruru. Диагностика теперь проверяет тип legacy-сети и её присутствие в списке, а не только состояние подключения; ответ null отмечается как непроверенный результат вместо успешной проверки.
