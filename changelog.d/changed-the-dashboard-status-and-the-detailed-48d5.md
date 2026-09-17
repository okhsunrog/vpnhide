_2026-09-15_

## English

The Dashboard status and the detailed diagnostics now say what the results mean and show the same state. While the VPN state or the hiding checks are being re-checked, the status card shows a neutral "Checking…" with a progress indicator instead of claiming "VPN hidden" or flashing "Couldn't determine whether VPN Hide is routed through the VPN" on a VPN toggle; a background re-read that finishes quickly changes nothing on screen. When the status is not simply green, the card says why: a configuration change still applying, unknown VPN routing, an interrupted or failed check, results that predate a VPN or configuration change, a check that measured nothing, or a needed restart. A failed network read is reported as unverified rather than as a leak, and a blocked or failed check is no longer restarted over and over by cache refreshes.

## Русский

Статус на главном экране и подробная диагностика теперь объясняют, что значат результаты, и показывают одно и то же состояние. Пока перепроверяется состояние VPN или идут проверки скрытия, карточка статуса показывает нейтральное «Проверяем…» с индикатором вместо утверждения «VPN скрыт» или мигания «Не удалось определить, идёт ли VPN Hide через VPN» при переключении VPN; быстрое фоновое перечитывание ничего на экране не меняет. Если статус не просто зелёный, карточка говорит почему: изменение настроек ещё применяется, маршрутизация VPN не определена, проверка прервана или не выполнилась, результаты получены до смены VPN или настроек, проверка ничего не измерила, либо нужен перезапуск. Неудавшееся чтение сети отмечается как непроверенное, а не как утечка, а заблокированная или упавшая проверка больше не перезапускается по кругу при обновлении кэшей.
