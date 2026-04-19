_2026-04-20_

## English

Diagnostics and Protection tabs now open instantly after the first load. Previously each tab switch re-ran all root-shell probes (module detection, target/observer files, pm lookups, 500ms of check functions), now those results live in process-lifetime caches. Diagnostics specifically: checks run once when VPN is first detected active, then results are fixed for the process — hooks don't change mid-session, so re-running is pointless. When VPN is off, both Dashboard and Diagnostics show a shared "turn on VPN, then Retry" banner. Log-capture tools (debug logging toggle, logcat recorder, debug-zip export) are now always visible on Diagnostics, regardless of VPN state.

## Русский

Вкладки «Диагностика» и «Защита» открываются мгновенно после первой загрузки. Раньше каждый переход между вкладками заново гонял все root-запросы (детект модулей, target/observer файлы, pm-запросы, 500мс функций проверки) — теперь результаты живут в кэше до перезапуска процесса. Конкретно по диагностике: проверки запускаются один раз, когда VPN впервые обнаружен активным, дальше результат фиксируется на сессию — хуки не меняются в пределах процесса, прогонять заново бессмысленно. Когда VPN выключен, Обзор и Диагностика показывают общий баннер «включите VPN, нажмите Повторить». Инструменты сбора логов (переключатель отладочных логов, запись logcat, экспорт debug-зипа) теперь всегда доступны на Диагностике, независимо от состояния VPN.
