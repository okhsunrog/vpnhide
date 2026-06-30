_2026-06-30_

## English

KPM backend: fix garbled status/stats control-channel replies on APatch (KernelPatch) devices that could make the app fail to start with 'Startup preparation failed'. The .kpm is now built with the large code model (-mcmodel=large) so its string-literal addressing stays correct when KernelPatch loads the module far from the kernel.

## Русский

KPM-бэкенд: исправлены искажённые ответы канала управления (status/stats) на устройствах APatch (KernelPatch), из-за которых приложение могло не запускаться с ошибкой «Startup preparation failed». Модуль .kpm теперь собирается с большой моделью кода (-mcmodel=large), чтобы адресация строковых литералов оставалась корректной при загрузке модуля далеко от ядра.
