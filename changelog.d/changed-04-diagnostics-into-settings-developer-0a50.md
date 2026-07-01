_2026-07-01_

## English

Diagnostics now lives under Settings, alongside a new Developer section: a debug-logging preference (off by default to keep logcat quiet and save resources; when enabled it turns on verbose kmod dmesg output and LSPosed hook logs, which are also captured in debug exports) plus a toggle to mute version and changelog notices on dev builds. The dashboard now waits for the full protection-check set and shows an in-app loading state instead of holding the splash screen. Diagnostics also shows a distinct checks-failed retry state on a failed run instead of misreporting an active VPN as off.

## Русский

Диагностика перенесена в настройки, рядом с новым разделом «Для разработчиков»: переключатель отладочных логов (выключены по умолчанию, чтобы не засорять logcat и не тратить ресурсы; при включении дают подробные логи модуля ядра в dmesg и логи LSPosed-хуков, которые также попадают в отладочный экспорт) и переключатель, скрывающий уведомления о версиях и changelog на dev-сборках. Обзор теперь ждёт полный набор проверок защиты и при запуске показывает загрузку внутри приложения вместо удержания splash-экрана. Диагностика также показывает отдельный баннер «проверки не выполнились, повторите» при ошибке прогона, а не сообщает ложно, что активный VPN выключен.
