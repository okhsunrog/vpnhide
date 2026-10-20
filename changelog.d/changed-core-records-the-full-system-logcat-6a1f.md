_2026-10-20_

## English

VPN Hide Core now records the full-system logcat itself. It keeps one continuous `logcat` running from Start to Stop, so nothing falls through 10-second gaps. Core stops the recording on its own at 64 MiB or after 30 minutes, and the Diagnostics card says when that happened. If the app is closed or restarted mid-recording, the card picks the running recording back up so you can stop and save it. Needs the matching VPN Hide Core update.

## Русский

VPN Hide Core теперь сам записывает полный системный logcat. От «Старт» до «Стоп» работает один непрерывный `logcat`, поэтому ничего не теряется в 10-секундных промежутках. Core сам останавливает запись на 64 МиБ или через 30 минут, и карточка в «Диагностике» сообщает, если так случилось. Если приложение закрыли или перезапустили во время записи, карточка снова подхватывает идущую запись, и её можно остановить и сохранить. Нужно соответствующее обновление VPN Hide Core.
