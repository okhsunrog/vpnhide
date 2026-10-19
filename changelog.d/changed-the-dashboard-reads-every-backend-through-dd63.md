_2026-10-19_

## English

The dashboard reads every backend through core: KPM liveness comes from core's own KPM status read, a backend module is checked for the payload core loads instead of an activator, the built-in kernel backend needs no companion module, and the Ports card and version checks follow the core module. Core now loads kmod, KPM and Zygisk at boot itself.

## Русский

Панель получает состояние всех бэкендов через core: активность KPM определяется собственным чтением статуса KPM в core, у модуля бэкенда проверяется загружаемый core файл, а не активатор, встроенному в ядро бэкенду не нужен companion-модуль, а карточка портов и проверка версий опираются на модуль core. Теперь core сам загружает kmod, KPM и Zygisk при загрузке.
