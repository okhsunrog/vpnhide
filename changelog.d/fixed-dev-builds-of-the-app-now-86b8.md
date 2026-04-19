_2026-04-19_

## English

Dev builds of the app now correctly receive "new version available" notifications. The comparison used to bail on the git-describe suffix (0.6.2-14-gSHA) and silently treat it as "no update", so testers running dev APKs never saw release prompts.

## Русский

Dev-сборки приложения теперь корректно получают уведомления о новой версии. Раньше сравнение спотыкалось о git-describe-суффикс (0.6.2-14-gSHA) и молча трактовало это как «обновлений нет», поэтому тестеры на dev-APK не видели релизных уведомлений.
