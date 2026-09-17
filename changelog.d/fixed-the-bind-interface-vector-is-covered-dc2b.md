_2026-08-24_

## English

The bind-to-interface vector is covered again on kernels below 5.9. The kernel backends only hooked the resolved-ifindex helper on 5.7–5.8 and otherwise relied on the kernel refusing an unprivileged bind, which a LineageOS 5.4 build did not; the helper is now found by symbol, and when a kernel exposes neither, the backend no longer claims to cover the vector. The Zygisk backend likewise decided from the kernel version and stayed silent where that assumption was wrong; it now measures what the running kernel answers for an interface that does not exist and gives a hidden one exactly the same answer.

## Русский

Вектор привязки сокета к интерфейсу снова закрыт на ядрах ниже 5.9. Ядерные бэкенды ставили хук только на ядра 5.7–5.8, а в остальных случаях полагались на то, что ядро само отвергнет привязку без прав, чего сборка LineageOS 5.4 не делала; теперь нужная функция ищется по имени в символах ядра, а если её нет вовсе, бэкенд больше не отчитывается о защите этого вектора. Zygisk-бэкенд так же решал по версии ядра и молчал там, где это предположение не выполнялось; теперь он измеряет, что работающее ядро отвечает на несуществующий интерфейс, и отдаёт скрытому ровно такой же ответ.
