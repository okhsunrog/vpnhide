_2026-04-19_

## English

Dev builds of the app no longer trigger a false 'module version mismatch' warning on the Dashboard. The check now strips the git-describe dev suffix (e.g. 0.6.2-14-g1f2205e vs module 0.6.2) before comparing.

## Русский

Сборки приложения из ветки больше не вызывают ложное предупреждение «версия модуля не совпадает» в Обзоре. Теперь сравнение отбрасывает суффикс git describe (0.6.2-14-g1f2205e → 0.6.2).
