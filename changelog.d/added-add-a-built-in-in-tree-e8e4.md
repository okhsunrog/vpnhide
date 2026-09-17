_2026-09-02_

## English

A built-in (in-tree, CONFIG_VPNHIDE=y) kernel backend and its companion module for kernels the loadable .ko cannot run on: integrated-modules, no-kprobes or whole-program-LTO kernels (e.g. Sultan). Same hiding coverage as the .ko, compiled into the kernel. The app recognises the built-in backend, picks its matching companion for activation, explains a redundant or missing companion, and refuses to load or configure the .ko over a live built-in backend.

## Русский

Добавлен встроенный бэкенд ядра (in-tree, CONFIG_VPNHIDE=y) и модуль-компаньон к нему — для ядер, на которых не запускается загружаемый .ko: интегрированные модули, отсутствие kprobes, LTO-ядра вроде Sultan. Скрывает те же вектора, что и .ko, но вкомпилирован в ядро. Приложение распознаёт встроенный бэкенд, само выбирает подходящий компаньон для активации, объясняет лишний или отсутствующий компаньон и не даёт загрузить или настроить .ko поверх работающего встроенного бэкенда.
