_2026-04-26_

## English

Restore debug-logging flag for the zygisk module on app start. KSU/Magisk replace the module dir wholesale on reinstall, wiping the runtime flag file even though the user's preference is still ON; the app now re-applies the persisted preference to disk on every start so logs stay enabled.

## Русский

Восстанавливаем флаг отладочного логирования для zygisk-модуля при старте приложения. KSU/Magisk при переустановке модуля заменяют его папку целиком, удаляя runtime-файл флага, хотя в настройках логирование всё ещё включено; приложение теперь переписывает сохранённое значение на диск при каждом старте, так что логи остаются включёнными.
