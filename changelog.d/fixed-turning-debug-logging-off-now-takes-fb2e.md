_2026-08-26_

## English

Turning Debug logging off now takes effect in the LSPosed hooks without a reboot. The flag was refreshed only by a filesystem watcher, and when that watcher stopped delivering events the hooks kept writing to logcat for as long as the device stayed up.

## Русский

Выключение отладочного лога теперь доходит до LSPosed-хуков без перезагрузки. Раньше флаг обновлялся только по событию файловой системы, и если это событие терялось, хуки продолжали писать в logcat до перезагрузки устройства.
