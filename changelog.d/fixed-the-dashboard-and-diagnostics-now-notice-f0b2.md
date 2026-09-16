_2026-09-16_

## English

The Dashboard and Diagnostics now notice a VPN turning on or off without a manual refresh. The VPN watcher never received a callback: its request carried the default NOT_VPN capability, and VPN Hide's own Java hook hides VPN callbacks from the app itself. It now also watches the default network, which the hook sanitizes but delivers.

## Русский

Панель и диагностика теперь замечают включение и выключение VPN без ручного обновления. Наблюдатель VPN не получал ни одного колбэка: его запрос нёс стандартную capability NOT_VPN, а собственный Java-хук VPN Hide скрывает VPN-колбэки от самого приложения. Теперь он следит и за сетью по умолчанию, которую хук санитизирует, но доставляет.
