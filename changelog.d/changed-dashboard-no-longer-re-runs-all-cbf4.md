_2026-04-19_

## English

Dashboard no longer re-runs all checks (module detection, kprobes, SELinux, target counts, GitHub update check, etc.) on every tab switch. State is loaded once at startup and cached in RAM; the Refresh button in the top bar forces a reload. Update check is cached for 6 hours and re-runs when the app comes back to the foreground after sitting in the background longer than that — no background jobs, no notifications, just reactive to app lifecycle.

## Русский

Обзор больше не перезапускает все проверки (обнаружение модулей, kprobes, SELinux, счётчики целей, проверка обновлений на GitHub и т. д.) при каждом переключении вкладки. Состояние загружается один раз при старте и кэшируется в памяти; кнопка «Обновить» в верхнем баре принудительно перезагружает. Проверка обновлений кэшируется на 6 часов и перезапускается при возврате приложения из фона, если прошло больше — никаких фоновых задач, никаких уведомлений, только реакция на жизненный цикл приложения.
