_2026-09-15_

## English

Bug-report bundles and the agent bridge now include the same diagnostic summary the app shows: whether the last check still applies, whether it was interrupted, and why a check cannot run right now. Collecting a debug log also no longer fails silently when the app cannot tell whether its own traffic goes through the VPN: the archive is written either way and records how the self-test ended and which run the results came from.

## Русский

Отчёты об ошибках и агентский мост теперь содержат ту же сводку диагностики, что показывает приложение: относится ли последняя проверка к текущему состоянию, была ли она прервана и почему проверка сейчас не может запуститься. Сбор отладочного лога также больше не срывается молча, когда приложение не может определить, идёт ли его собственный трафик через VPN: архив собирается в любом случае, и в нём записано, чем закончилась самопроверка и из какого запуска взяты результаты.
