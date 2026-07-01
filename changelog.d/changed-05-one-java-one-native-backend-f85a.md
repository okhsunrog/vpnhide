_2026-07-01_

## English

The Dashboard now models your setup as one Java backend (LSPosed) and one Native backend (kmod, KPM, or Zygisk) instead of a per-module list. It recognizes the new KPM backend and warns when more than one native backend is installed — an error for the kmod+KPM combination, which can freeze the kernel — and KPM reports a truthful conflict status when it stands down for a co-installed kernel module.

## Русский

«Обзор» теперь представляет конфигурацию как один Java-бэкенд (LSPosed) и один нативный бэкенд (kmod, KPM или Zygisk), а не список модулей. Он распознаёт новый бэкенд KPM и предупреждает, когда установлено больше одного нативного бэкенда — для связки kmod+KPM это ошибка, она может подвесить ядро, — а KPM честно сообщает о конфликте, когда отключается из-за уже установленного модуля ядра.
