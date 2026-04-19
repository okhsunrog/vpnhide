_2026-04-19_

## English

App Hiding: marking the same app as both H (Hidden) and O (Observer) caused it to crash on startup — the app would query its own PackageInfo, our system_server hook matched it as an observer and stripped its own package from the result, and the framework bailed. Roles are now mutually exclusive: toggling one clears the other, and existing H+O configs are migrated to O-only on first launch.

## Русский

Скрытие приложений: если приложение было отмечено одновременно как H (скрытое) и O (observer), оно падало при запуске — запрашивало свою собственную PackageInfo, наш хук в system_server вырезал его из ответа как наблюдателю, и фреймворк ломался. Роли теперь взаимоисключающие: включение одной сбрасывает другую, а уже сохранённая комбинация H+O автоматически превращается в O при первом запуске.
