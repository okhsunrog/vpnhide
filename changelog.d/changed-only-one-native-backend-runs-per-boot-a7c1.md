_2026-10-19_

## English

Only one native backend runs per boot. Core picks it once at boot from the installed kmod, KPM and Zygisk modules plus a built-in or already-running kernel backend. If more than one is present it loads none of them, and the dashboard says which ones are installed and asks you to keep exactly one. A module staged for the next reboot that would cause a second backend gets a warning before you reboot. The KPM SuperKey, KPM and kernel-module load-failure banners work again, now driven by core's status. Saving a SuperKey loads a waiting KPM without a reboot.

## Русский

За одну загрузку работает только один нативный бэкенд. Core выбирает его один раз при загрузке среди установленных модулей kmod, KPM и Zygisk, а также встроенного или уже работающего бэкенда ядра. Если их больше одного, не загружается ни один, а панель показывает, какие установлены, и просит оставить ровно один. Если модуль, ожидающий перезагрузки, добавит второй бэкенд, вы увидите предупреждение ещё до перезагрузки. Снова работают предупреждения о SuperKey для KPM и об ошибках загрузки KPM и модуля ядра; теперь они берутся из статуса core. После сохранения SuperKey ожидающий KPM загружается без перезагрузки.
