_2026-11-03_

## English

Temporary debug logging and logcat recording now count time spent suspended toward their limits. A clock failure stops recording with an explicit reason while preserving the last measured duration and normal logging restoration. Requires paired APK/Core RPC31; earlier state migrates to journal14, which older Core builds refuse to modify.

## Русский

Время сна телефона теперь учитывается в ограничениях временного debug logging и записи logcat. При сбое часов запись останавливается с явной причиной, сохраняя последнюю измеренную длительность и обычное восстановление logging. Требуется совместное обновление APK/Core до RPC31; прежнее состояние переносится в journal14, который старые версии Core откажутся изменять.
