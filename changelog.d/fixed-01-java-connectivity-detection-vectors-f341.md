_2026-07-01_

## English

Closed several Java Connectivity detection vectors so more apps can no longer see the VPN: network-callback pushes (e.g. VTB), getNetworkInfo(TYPE_VPN) (e.g. Улыбка радуги), the legacy NetworkInfo API (getActiveNetworkInfo), and the VPN Network handles from getActiveNetwork/getAllNetworks are now sanitized for target apps. The system_server hooks scrub results through public APIs, so they also cover Android 17, which changed the private fields the old hooks read.

## Русский

Целевые приложения больше не могут обнаружить VPN через Java Connectivity API: теперь скрываются push-данные сетевых колбэков (например, ВТБ), getNetworkInfo(TYPE_VPN) (например, Улыбка радуги), устаревший API NetworkInfo (getActiveNetworkInfo) и VPN-дескрипторы Network из getActiveNetwork/getAllNetworks. Хуки в system_server чистят результаты через публичные API, поэтому работают и на Android 17 (он изменил приватные поля, которые читали старые хуки).
