_2026-08-24_

## English

The Zygisk backend no longer risks crashing a target app that passes a bad pointer to an interface ioctl. The interface name is now read through the same fault-contained path the setsockopt hook uses, so such a call gets the kernel's own EFAULT instead of a segfault inside the app.

## Русский

Zygisk-бэкенд больше не может уронить целевое приложение, если оно передаёт некорректный указатель в ioctl по интерфейсу. Имя интерфейса читается тем же защищённым от сбоя способом, что и в хуке setsockopt, поэтому такой вызов получает штатный EFAULT от ядра, а не segfault внутри приложения.
