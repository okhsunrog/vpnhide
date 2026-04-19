_2026-04-19_

## English

Dashboard now splits issues into Errors (red, block protection) and Warnings (amber, setup is suboptimal but working). Four new warnings: kernel supports kmod but only Zygisk is installed; kmod and Zygisk both active simultaneously (means you have to remember per-app Z-off for banking / payment apps that detect Zygisk); debug logging left on; SELinux in Permissive mode (exposes six detection vectors that VPN Hide relies on the kernel to block).

## Русский

Обзор теперь делит проблемы на Ошибки (красные, ломают защиту) и Предупреждения (жёлтые, защита работает, но конфигурация не оптимальная). Четыре новых предупреждения: ядро поддерживает kmod, но установлен только Zygisk; kmod и Zygisk активны одновременно (значит, для каждого банковского или платёжного приложения придётся помнить выключать Z вручную, иначе они его обнаруживают); оставлены включёнными отладочные логи; SELinux в режиме Permissive (открывает шесть векторов детекции, которые должен блокировать).
