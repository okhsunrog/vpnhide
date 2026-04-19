_2026-04-19_

## English

Debug logging toggle in Diagnostics: off by default — VPN Hide, LSPosed hooks (VpnHide-NC/NI/LP and the package-visibility filter), and zygisk keep logcat near-silent. Start recording and Collect debug log automatically enable verbose logging for the duration of the capture and restore it afterwards, so the toggle is only needed if you want logs emitted continuously outside a capture. Errors always pass through so hook-install failures remain visible.

## Русский

Переключатель отладочных логов в Диагностике: по умолчанию выключен — приложение, хуки LSPosed (VpnHide-NC/NI/LP, фильтр видимости пакетов) и zygisk почти ничего не пишут в logcat. Кнопки «Начать запись» и «Собрать отладочный лог» автоматически включают логирование на время захвата и возвращают состояние обратно, так что переключатель нужен только если вы хотите, чтобы логи писались постоянно, вне сессии захвата. Ошибки всегда видны, чтобы проблемы установки хуков не терялись.
