_2026-08-24_

## English

The bind-interface vector is covered on kernels below 5.9 again. Backends only hooked the resolved-ifindex helper on 5.7-5.8 and otherwise relied on the kernel refusing an unprivileged bind — a LineageOS 5.4 build let an app bind a socket to the VPN interface anyway. The helper is now found by symbol, and when a kernel exposes neither, the backend no longer claims to cover the vector.

## Русский

Вектор привязки сокета к интерфейсу снова закрыт на ядрах ниже 5.9. Раньше хук ставился только на ядра 5.7-5.8, а в остальных случаях мы полагались на то, что ядро само отвергнет привязку без прав — на сборке LineageOS 5.4 приложение всё равно смогло привязаться к VPN-интерфейсу. Теперь нужная функция ищется по имени в символах ядра, а если её нет вовсе, бэкенд больше не отчитывается о защите этого вектора.
