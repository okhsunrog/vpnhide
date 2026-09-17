_2026-09-17_

## English

The Dashboard and Diagnostics now follow VPN changes reliably. Turning the VPN on or off is noticed without a manual refresh, and re-adding VPN Hide to a tunnel (or a tunnel re-established) while the app was in the background re-checks hiding on its own when you return, instead of stopping at "Re-check needed" or flashing a re-check state; the Dashboard tiles follow that check. The app no longer depends on network callbacks for this, which its own Java hook hides from it: while it is open it observes its own VPN state directly.

## Русский

Главный экран и диагностика теперь надёжно следят за изменениями VPN. Включение и выключение VPN замечаются без ручного обновления, а если VPN Hide вернули в туннель (или туннель пересоздался), пока приложение было в фоне, при возврате проверка скрытия запускается сама, а не останавливается на «Нужна повторная проверка» и не мигает перепроверкой; плитки на главном экране обновляются вместе с ней. Приложение больше не полагается для этого на сетевые события, которые его собственный Java-хук от него скрывает: пока оно открыто, оно наблюдает своё состояние VPN напрямую.
