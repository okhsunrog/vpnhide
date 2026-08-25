_2026-08-25_

## English

The Zygisk backend now hides interface binds on kernels where it previously stayed silent. It used to decide from the kernel version, assuming older kernels refuse an unprivileged bind on their own; where that is not true, the interface stayed bindable. It now measures what the running kernel answers for an interface that does not exist and gives a hidden one exactly the same answer.

## Русский

Zygisk-бэкенд теперь скрывает привязку к интерфейсу на ядрах, где раньше молчал. Решение принималось по версии ядра, исходя из того, что старые ядра сами отвергают привязку без прав; там, где это не так, интерфейс оставался доступным. Теперь бэкенд измеряет, что работающее ядро отвечает на несуществующий интерфейс, и отдаёт скрытому ровно такой же ответ.
