_2026-11-03_

## English

KPM delivery no longer fails with a busy control lock after a local observation finished while another child process inherited its descriptor. Lock ownership ends with the original scope; external contention and uncertain delivery still prevent retries.

## Русский

Доставка настроек KPM больше не получает ошибку занятой блокировки после завершённого локального наблюдения, если другой дочерний процесс унаследовал его дескриптор. Блокировка освобождается владельцем; внешняя конкуренция и неопределённый результат по-прежнему запрещают повтор операции.
