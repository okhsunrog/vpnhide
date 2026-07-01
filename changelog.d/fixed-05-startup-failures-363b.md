_2026-07-01_

## English

Fixed the app failing to start with "Startup preparation failed" — the root setup command was being flattened to one line, breaking its shell syntax. The failure screen now explains the actual cause (root unavailable, incomplete root data, or a config-write failure) and shows the underlying error detail instead of always telling you to check root permissions.

## Русский

Исправлен запуск приложения с ошибкой «Startup preparation failed» — команда настройки root схлопывалась в одну строку и ломала синтаксис shell. Экран ошибки теперь объясняет настоящую причину (нет root, неполные root-данные или ошибка записи конфига) и показывает техническую деталь ошибки, вместо того чтобы всегда советовать проверить права root.
