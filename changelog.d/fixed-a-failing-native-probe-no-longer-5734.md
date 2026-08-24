_2026-08-24_

## English

A failing native probe no longer takes the app down with it. The probe library now unwinds instead of aborting, so an unexpected kernel reply surfaces as one failed check run — with the panic message and its source line in logcat — instead of killing the process at startup.

## Русский

Сбой нативной проверки больше не роняет приложение. Библиотека проверок теперь разворачивает стек вместо аварийного завершения: неожиданный ответ ядра приводит к одной неудачной проверке (с сообщением и строкой исходника в logcat), а не к падению приложения при запуске.
