_2026-06-30_

## English

KPM interception stats now appear on KernelSU / KPatch-Next setups. The activator treated the kpatch CLI's exit status (which is the reply byte count for a ctl0 read, not 0) as a read failure, so the dashboard's KPM status/stats came back empty.

## Русский

Статистика перехватов KPM теперь отображается на KernelSU / KPatch-Next. Активатор воспринимал код выхода CLI kpatch (для чтения ctl0 это число байт ответа, а не 0) как ошибку чтения, из-за чего статус/статистика KPM на дашборде были пустыми.
