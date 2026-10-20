_2026-10-20_

## English

A `vpnhide.kpm` you load yourself (from the APatch/FolkPatch KPM screen, with KPatch-Next, or embedded in the boot image) is now a supported way to install the KPM. Core checks its version, reads its filesystem-hiding setting and configures it the same way as the module. The "unsupported KPM installation" warning is gone. If the running KPM is older than 1.2.0, the Dashboard now shows a clear "too old" error instead of the KPM silently getting no settings. If the KPM module is installed and the same KPM is already running, core uses the running copy and does not load a second one.

## Русский

Самостоятельно загруженный `vpnhide.kpm` (с экрана KPM в APatch/FolkPatch, через KPatch-Next или встроенный в boot-образ) теперь поддерживаемый способ установки KPM. Core проверяет его версию, читает настройку скрытия в файловой системе и настраивает его так же, как модуль. Предупреждение «неподдерживаемая установка KPM» убрано. Если работающий KPM старше 1.2.0, панель показывает понятную ошибку «слишком старый», а не оставляет KPM без настроек молча. Если установлен модуль KPM и тот же KPM уже работает, core использует работающую копию и вторую не загружает.
