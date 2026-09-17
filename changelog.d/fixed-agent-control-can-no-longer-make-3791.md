_2026-09-16_

## English

Agent control can no longer make VPN Hide itself a target of the ports module. That setting rejected the app's own localhost connections, which cut the agent bridge off until the change was reverted by hand, and could crash the app when the bridge's client disconnected mid-request. VPN Hide's own roles are now fixed on every write path, and a client disconnecting mid-request is handled.

## Русский

Agent control больше не может сделать сам VPN Hide целью модуля портов. Такая настройка блокировала localhost-соединения самого приложения, отрезала агентский мост, пока изменение не откатывали вручную, и могла уронить приложение, если клиент моста отключался посреди запроса. Теперь роли самого VPN Hide фиксированы на всех путях записи, а отключение клиента посреди запроса обрабатывается.
