_2026-07-01_

## English

The dashboard now explains more failure modes instead of surfacing raw errors: a kernel that rejects the module because it enforces module signatures (EKEYREJECTED — it recommends KernelSU Next), a KPM installed but waiting for the APatch superkey, and a hiding layer that is active while some of its runtime checks still fail (a Details button opens the full diagnostics). A fresh install with no targets now reads as guidance rather than an error. It also detects private AOSP fields broken by a new Android release at install time, listing the affected fields and Android SDK so you can file a bug.

## Русский

Дашборд теперь поясняет больше причин сбоя вместо непонятных ошибок: модуль ядра отклонён из-за обязательной подписи модулей (EKEYREJECTED — рекомендуется KernelSU Next); KPM установлен, но неактивен и ждёт суперключ APatch; слой скрытия активен, но часть его проверок не проходит — тогда показывается предупреждение с кнопкой «Подробности», открывающей полную диагностику. Свежая установка без выбранных целей теперь показывается как подсказка, а не ошибка. При установке также проверяются приватные AOSP-поля: если новая версия Android их переименовала или сменила тип, дашборд показывает ошибку со списком сломанных полей и версией Android, чтобы можно было завести issue.
