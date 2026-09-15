_2026-09-15_

## English

The dashboard appears about twice as fast on a cold start (roughly 3 s to 1.5 s on a Pixel 8 Pro): the app reads the root state once instead of two or three times, the first root shell doubles as the root check, the privileged config session opens in one round trip, and the startup runtime re-activation runs after the first screen is shown.

## Русский

Панель при холодном запуске появляется примерно вдвое быстрее (около 3 с → 1,5 с на Pixel 8 Pro): приложение читает root-состояние один раз вместо двух-трёх, первая root-команда одновременно служит проверкой root, привилегированная сессия конфигурации открывается за одно обращение, а повторная активация при старте выполняется после показа первого экрана.
