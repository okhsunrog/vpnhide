_2026-08-22_

## English

Diagnostics now explain a partially loaded native backend instead of leaving a red leak unexplained: the dashboard says how many kernel hooks installed and which ones the kernel did not expose, and the leaking check names the hook that is missing. A KPM whose control call is refused while no SuperKey is saved now says so, instead of suggesting a reinstall that cannot help.

## Русский

Диагностика теперь объясняет частично загрузившийся нативный бэкенд, а не оставляет красную утечку без объяснения: на обзоре видно, сколько хуков ядра встало и каких функций ядро не отдало, а в самой проверке названы недостающие хуки. Если у KPM отклонён управляющий вызов и при этом не сохранён SuperKey, приложение так и говорит, вместо совета переустановить модуль, который тут не поможет.
