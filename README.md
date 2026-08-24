<p align="center">
  <img src="assets/logo.png" width="200" alt="VPN Hide" />
</p>

<h1 align="center">VPN Hide</h1>

<p align="center">Скрывает активное VPN-соединение на Android от выбранных приложений.</p>

<p align="center">
  <a href="https://github.com/okhsunrog/vpnhide/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/okhsunrog/vpnhide/ci.yml?label=CI" alt="CI"></a>
  <a href="https://github.com/okhsunrog/vpnhide/releases/latest"><img src="https://img.shields.io/github/v/release/okhsunrog/vpnhide" alt="Release"></a>
  <a href="https://github.com/okhsunrog/vpnhide/releases"><img src="https://img.shields.io/github/downloads/okhsunrog/vpnhide/total" alt="Downloads"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="License"></a>
</p>

<p align="center"><strong><a href="README.en.md">English version</a> · <a href="README.zh.md">中文版</a></strong></p>

## Чем vpnhide лучше аналогов?

Существующие модули, такие как [NoVPNDetect](https://bitbucket.org/yuri-project/novpndetect) и [NoVPNDetect Enhanced](https://github.com/BlueCat300/NoVPNDetectEnhanced), покрывают только **Java API** обнаружение и хукают **внутри процесса** целевого приложения через Xposed. У этого подхода две критические проблемы:

1. **Обнаружение anti-tamper защитой** — любое приложение с проверкой инъекций в память обнаруживает хуки Xposed и отказывается работать. Автор NoVPNDetect Enhanced прямо пишет: *«Модуль не будет работать если у подключаемого приложения есть защита от LSPosed, проверка на инъекции в память. Например MirPay, Т-Банк.»*
2. **Нет нативного покрытия** — приложения, использующие C/C++ код, кроссплатформенные фреймворки (Flutter, React Native) или прямые системные вызовы, могут обнаружить VPN через `ioctl`, `getifaddrs`, netlink-сокеты и `/proc/net/*`. Java-хуки эти векторы полностью пропускают.

vpnhide решает обе проблемы многослойной архитектурой:

**Уровень 1 — Java API (модуль lsposed):** хукает `system_server`, а не целевое приложение. `NetworkCapabilities`, `NetworkInfo` и `LinkProperties` фильтруются на уровне Binder *до того*, как данные попадут в процесс приложения. Приложение получает чистые данные через IPC — никаких инъекций в его процесс, нечего обнаруживать. Этот же модуль скрывает выбранные приложения от приложений-наблюдателей на уровне `PackageManager` (роль Apps).

**Уровень 2 — нативный (kmod, KPM или Zygisk):** покрывает нативные пути обнаружения. Активным должен быть ровно один Native-бэкенд:
- **kmod** (рекомендуется для поддерживаемых GKI-ядер) — хуки `kprobe`/`kretprobe` на уровне ядра. Фильтрует `ioctl`, `getifaddrs`/netlink-дампы интерфейсов, адресов, маршрутов и policy rules, а также отклоняет `SO_BINDTODEVICE` / `SO_BINDTOIFINDEX` для скрытых интерфейсов до изменения состояния сокета. Нулевой след в процессе приложения: никаких инъекций библиотек, нечего обнаруживать.
- **KPM** — KernelPatch Module с теми же 11 логическими kernel-хуками, но без привязки к GKI-варианту `.ko`. Полезен для старых/non-GKI ядер 4.9 / 4.14 / 4.19 / 5.4 и ситуаций, где `.ko` не может загрузиться. Требует KernelPatch runtime: APatch или KPatch-Next-Module.
- **Zygisk** — запасной вариант, если kernel-level backend поставить нельзя. Inline-хуки `libc.so`, включая best-effort фильтрацию `setsockopt`, работают внутри процесса приложения и обходятся прямыми системными вызовами, поэтому банковские и anti-fraud приложения могут их обнаруживать. Для таких приложений лучше оставлять Native выключенным и полагаться на Java-уровень.

**Уровень 3 — модуль скрытия портов (portshide):** отдельный Magisk-модуль. Через iptables блокирует выбранным приложениям доступ к `127.0.0.1` / `::1`, чтобы они не могли обнаружить локально запущенный VPN / proxy-демон по открытому порту (роль Ports).

Процесс целевого приложения полностью нетронут при использовании LSPosed + kernel-level Native-бэкенда (kmod или KPM) — ни Xposed, ни inline-хуков, ни модифицированных регионов памяти. Благодаря этому vpnhide работает с банковскими и государственными приложениями, которые активно обнаруживают и блокируют модули на основе Xposed.

## Что именно скрывает vpnhide

vpnhide скрывает от выбранных приложений три вещи — всё настраивается пер-апп через четыре роли **J / N / A / P** (Java, Native, Apps, Ports):

1. **Скрытие интерфейса** — главная задача. Убирает VPN-интерфейсы и маршруты из нативных API (`ioctl`, `getifaddrs`, netlink, `/proc/net/*`, `NetworkInterface`), не позволяет привязать сокет к скрытому интерфейсу и очищает Java API (`NetworkCapabilities`, `NetworkInfo`, `LinkProperties`). Его дают две роли сразу — **Java (J)** и **Native (N)**, они включаются независимо.
2. **Скрытие портов** — блокирует доступ к localhost для выбранных приложений, чтобы они не могли обнаружить Clash, sing-box, V2Ray, Happ и подобные инструменты через проверку локальных портов (роль **Ports (P)**).
3. **Скрытие приложений** — позволяет скрыть выбранные установленные приложения от выбранных приложений-наблюдателей. Полезно против проверок package visibility, например когда приложение пытается определить, установлен ли на устройстве VPN или proxy-клиент (роль **Apps (A)**).

## Какие модули нужны?

Всегда нужно **приложение VPN Hide** (`vpnhide.apk`) + LSPosed/Vector для Java-уровня + ровно один Native-бэкенд для нативного скрытия. Дополнительно приложение может использовать необязательный Ports-модуль для блокировки localhost-портов:

- **`kmod`** (стабильный вариант по умолчанию) — полностью out-of-process, невидим для anti-tamper. Требуется поддерживаемое GKI-ядро: 5.10, 5.15, 6.1, 6.6 или 6.12.
- **`KPM`** — kernel-level backend для 4.9 / 4.14 / 4.19 / 5.4 и других случаев, где `.ko` не подходит. Нужен APatch или KPatch-Next-Module.
- **`Zygisk`** — fallback, если kmod/KPM недоступны или вы не хотите ставить KernelPatch runtime.
- **`portshide`** (необязательно) — установите, если хотите блокировать выбранным приложениям доступ к localhost-портам.

Не ставьте несколько Native-бэкендов одновременно. Если они всё же установлены, приложение выбирает активный по приоритету: kmod, затем KPM, затем Zygisk; лишние модули лучше удалить.

См. [Установка](#установка) для пошаговой инструкции.

## Установка

Скачайте последний релиз из [Releases](https://github.com/okhsunrog/vpnhide/releases).

### Шаг 1 — Приложение VPN Hide + LSPosed

1. Установите `vpnhide.apk` как обычное приложение
2. В менеджере LSPosed включите модуль VPN Hide и добавьте **«System Framework»** в его область действия
3. Перезагрузите устройство (обязательно — хуки LSPosed внедряются в `system_server` при загрузке, поэтому модуль должен быть активен до запуска `system_server`)
4. Откройте приложение VPN Hide и предоставьте ему root-доступ (Magisk обычно запросит автоматически; на KernelSU/KernelSU-Next/APatch выдайте разрешение в менеджере)

### Шаг 2 — Нативный модуль для скрытия интерфейса

Откройте приложение VPN Hide. На вкладке **«Обзор»** приложение определит устройство и ядро и покажет, какой Native-бэкенд лучше установить:

- Для поддерживаемого GKI-ядра будет рекомендован конкретный файл kmod, например `vpnhide-kmod-android14-6.1.zip`.
- Для non-GKI/старых ядер 4.9 / 4.14 / 4.19 / 5.4 будет рекомендован `vpnhide-kpm.zip`. Если KernelPatch runtime ещё не найден, приложение попросит сначала установить KPatch-Next-Module или использовать Zygisk как fallback.
- Для остальных ядер будет рекомендован `vpnhide-zygisk.zip`.

Установите рекомендованный модуль:
- **kmod:** через KernelSU-Next / KernelSU / Magisk → Модули → Установить из хранилища.
- **KPM:** установите `vpnhide-kpm.zip`; под APatch/FolkPatch приложение может попросить сохранить SuperKey в **Настройки → Безопасность** для активации при загрузке, если runtime не выдаёт доверенный `su`-токен KernelPatch. Под Magisk, KernelSU и KernelSU-Next сначала установите KPatch-Next-Module, если он ещё не установлен.
- **Zygisk:** через KernelSU-Next, KernelSU или Magisk → Модули.

Перезагрузите устройство после установки нативного модуля.

### Шаг 3 — Необязательно: установка Ports-модуля

Если вам нужна блокировка localhost-портов, установите `vpnhide-ports.zip` через KernelSU-Next или Magisk manager.

Этот модуль независим от Native-бэкенда и нужен только для роли **Ports** в приложении.

### Шаг 4 — Настройка скрытия

Откройте приложение VPN Hide → вкладка **«Скрытие»**.

В списке у каждого приложения есть роли:

- **Java** — скрытие VPN через Android Java API на уровне LSPosed/system_server.
- **Native** — активный Native-бэкенд: kmod, KPM или Zygisk. VPN Hide сохраняет один выбор Native; работает только активный бэкенд.
- **Apps** — приложение становится наблюдателем, от которого нужно скрывать выбранные VPN/proxy-приложения через PackageManager.
- **Ports** — приложению блокируется доступ к localhost-портам.

В настройках можно включить полные подписи ролей вместо коротких **J / N / A / P**. Для Java, Native и Ports значок настройки рядом с названием открывает индивидуальные хуки или диапазоны портов.

После изменений нажмите «Сохранить».

#### Кто на кого настраивается

Роли ставятся на **приложение-охотник** — то, от которого вы прячете VPN (банк, госуслуги, маркетплейс). Само VPN-приложение в этом списке настраивать не нужно: оно выступает в роли «жертвы», и то, что нужно спрятать *его самого*, задаётся в Настройки → «Скрытие VPN-приложений».

Типовые сценарии:

| Что происходит | Что включить |
|---|---|
| Банк не должен видеть, что включён VPN | На **банке**: Java + Native |
| Банк ещё и проверяет список установленных приложений | Дополнительно **Apps** на банке. Затем в Настройки → «Скрытие VPN-приложений» убедиться, что ваш VPN там есть: приложения с VpnService находятся автоматически, остальные добавляются вручную |
| Банк ругается именно на Zygisk | Native выключить, оставить Java. На GKI-устройстве лучше перейти на kmod или KPM — они не видны изнутри процесса |
| Приложение стучится в localhost-порт прокси (127.0.0.1:1080 и подобные) | Дополнительно **Ports** на этом приложении (нужен установленный ports-модуль) |

Пример: чтобы банковское приложение не видело ни VPN, ни установленный WireGuard, включите ему **J + N + A** и проверьте, что WireGuard отмечен в списке скрываемых. Самому WireGuard роли не нужны.


Java и kernel-level Native-бэкенды (kmod/KPM) применяются сразу. Zygisk-хуки и правила Ports применяются для выбранного приложения после принудительной остановки и повторного запуска.

> **Примечание:** некоторые приложения обнаруживают Zygisk-хуки, если для них включён Native. Для таких приложений оставьте Native выключенным и используйте Java-уровень, либо перейдите на kmod/KPM.

<details>
<summary><b>Настройка через командную строку (для продвинутых)</b></summary>

Пользовательская конфигурация хранится в `/data/system/vpnhide_config.json`. Отредактируйте JSON и запустите activator установленного модуля:

```sh
su -c /data/adb/modules/vpnhide_kmod/activator
su -c /data/adb/modules/vpnhide_kpm/activator
su -c /data/adb/modules/vpnhide_zygisk/activator
su -c /data/adb/modules/vpnhide_ports/activator
```

Запускайте только activator тех модулей, которые действительно установлены. LSPosed читает JSON напрямую из `system_server`, отдельный activator для него не нужен. Старые `targets.txt` в `/data/adb/vpnhide_*` — это конфиг версий до 1.0. Пользовательской настройкой они больше не являются: приложение один раз переносит их в JSON и удаляет.

</details>

<details>
<summary><b>Ручной подбор GKI (если хотите выбрать файл kmod самостоятельно)</b></summary>

1. На телефоне откройте **Настройки → О телефоне** и найдите строку **Версия ядра**. Она выглядит примерно так: `6.1.75-android14-11-g...`
2. Вам нужны две части из этой строки: версия ядра (`6.1`) и поколение android (`android14`). Вместе они образуют ваше поколение GKI: `android14-6.1`
3. Скачайте соответствующий файл из релиза: `vpnhide-kmod-android14-6.1.zip`

Также можно выполнить `adb shell uname -r` через ADB, чтобы увидеть строку версии ядра.

> **Важно:** `android14` в строке ядра — это НЕ версия Android, а поколение ядра. Например, все Pixel с 6 по 9a используют ядро `android14-6.1` вне зависимости от того, стоит ли на них Android 14 или 15.

</details>

## Скриншоты

| Обзор — VPN скрыт | Скрытие — единый список | Как это работает |
|:-:|:-:|:-:|
| <img src="assets/screenshots/dashboard-hidden.png" width="250"> | <img src="assets/screenshots/hiding-list.png" width="250"> | <img src="assets/screenshots/hiding-help.png" width="250"> |

| Статистика | Разбор по хукам | Выбор хуков для приложения |
|:-:|:-:|:-:|
| <img src="assets/screenshots/statistics.png" width="250"> | <img src="assets/screenshots/statistics-breakdown.png" width="250"> | <img src="assets/screenshots/hook-picker.png" width="250"> |

| Диагностика | Настройки | Сообщество |
|:-:|:-:|:-:|
| <img src="assets/screenshots/diagnostics-native.png" width="250"> | <img src="assets/screenshots/settings.png" width="250"> | <img src="assets/screenshots/community.png" width="250"> |

## Проверка

В приложении есть встроенная система диагностики, которая автоматически обнаруживает большинство проблем с настройкой.

**Обзор** (запускается при каждом открытии приложения):
- Статус модулей и бэкендов (установлен, активен, версия, количество целей)
- Валидация конфигурации LSPosed — читает базу данных LSPosed и проверяет, что VPN Hide включён, System Framework в scope, и нет лишних приложений в scope (частая ошибка при настройке)
- Обнаружение несоответствия версий — сравнивает версии установленных модулей с версией приложения и подсказывает, что именно нужно обновить
- Рекомендация Native-бэкенда — определяет ядро устройства и подбирает нужный файл kmod, KPM или Zygisk
- Проверка скрытия в реальном времени (при активном VPN) — выполняет 13 нативных и 12 Java API проверок, чтобы убедиться, что VPN действительно скрыт

Все обнаруженные проблемы показываются в виде карточек с конкретными инструкциями по исправлению.

**Статистика** — счётчики перехватов по каждому приложению: какие приложения и какими методами проверяют VPN.

**Настройки → Диагностика** — детальная разбивка по каждой проверке с индивидуальными результатами PASS/FAIL по всем 25 проверкам. Полезна для отладки, когда «Обзор» показывает неполное скрытие.

## Компоненты

| Директория | Что | Как |
|---|---|---|
| **[kmod/](kmod/)** | Модуль ядра `.ko` + KPM backend (C) | Два kernel-level Native-бэкенда: стабильный GKI `.ko` на `kretprobe` и KPM на KernelPatch inline hooks. Оба дают нулевой след в процессе приложения; активным должен быть только один. ([подробнее](kmod/README.md), [KPM](kmod/kpm/README.md)) |
| **[lsposed/](lsposed/)** | LSPosed-модуль + приложение (Kotlin + Rust) | Хуки `writeToParcel` в `system_server` для per-UID фильтрации Binder. APK предоставляет обзорную панель (статус модулей, проверка версий, валидация конфигурации LSPosed, рекомендации по установке), вкладку «Скрытие» для ролей Java / Native / Apps / Ports и диагностику. ([подробнее](lsposed/README.md)) |
| **[portshide/](portshide/)** | Модуль скрытия портов (Shell + iptables) | Блокирует выбранным приложениям доступ к `127.0.0.1` / `::1`, скрывая локально запущенные VPN / proxy-демоны от проверок localhost-портов. ([подробнее](portshide/README.md)) |
| **[zygisk/](zygisk/)** | Zygisk-модуль (Rust) | Inline-хуки `libc.so` в процессе приложения. Fallback, когда kernel-level backend недоступен. ([подробнее](zygisk/README.md)) |

## Покрытие обнаружения

| # | Вектор обнаружения | SELinux | kmod | KPM | Zygisk | LSPosed |
|---|---|---|---|---|---|---|
| 1 | `ioctl(SIOCGIFFLAGS)` на tun0 | | x | x | x | |
| 2 | `ioctl(SIOCGIFNAME)` разрешение индекса в имя | | x | x | x | |
| 3 | `ioctl(SIOCGIFMTU)` фингерпринтинг MTU | | x | x | x | |
| 4 | `ioctl(SIOCGIFCONF)` перечисление интерфейсов | | x | x | x | |
| 5 | Все остальные `SIOCGIF*` (INDEX, HWADDR, ADDR и т.д.) | | x | x | x | |
| 6 | `getifaddrs()` (использует netlink внутри) | | x | x | x | |
| 7 | netlink `RTM_GETLINK` дамп | | x | x | x | |
| 8 | netlink `RTM_GETADDR` дамп (IPv4 + IPv6) | | x | x | x | |
| 9 | netlink `RTM_GETROUTE` дамп | | x | x | x | |
| 10 | netlink `RTM_GETRULE` (policy rules) | | x | x | | |
| 11 | Публичный `/32` или `/128` host-route к VPN-серверу | | x | x | | |
| 12 | `SO_BINDTODEVICE` / `SO_BINDTOIFINDEX` | | x | x | libc | |
| 13 | `/proc/net/route` | блок. | x | x | x | |
| 14 | `/proc/net/ipv6_route` | блок. | x | x | x | |
| 15 | `/proc/net/if_inet6` | блок. | | | x | |
| 16 | `/proc/net/tcp`, `tcp6` | блок. | | | x | |
| 17 | `/proc/net/udp`, `udp6` | блок. | | | | |
| 18 | `/proc/net/dev` | блок. | | | | |
| 19 | `/proc/net/fib_trie` | блок. | | | | |
| 20 | `/sys/class/net/tun0/` и `/proc/sys/net/*/{conf,neigh}/tun0` | блок. | опц. | | | |
| 21 | `NetworkCapabilities` (hasTransport, NOT_VPN, transportInfo) | | | | | x |
| 22 | `NetworkInfo` (getType, getTypeName) | | | | | x |
| 23 | `ConnectivityManager.getActiveNetwork()` | | | | | x |
| 24 | `ConnectivityManager.getAllNetworks()` + VPN-сканирование | | | | | x |
| 25 | `LinkProperties` (interfaceName) | | | | | x |
| 26 | `LinkProperties` (маршруты через VPN-интерфейсы) | | | | | x |
| 27 | `NetworkInterface.getNetworkInterfaces()` | | x | x | x | |
| 28 | `/proc/net/route` через Java `FileInputStream` | блок. | x | x | x | |

**блок.** = на сток-enforcing сборках (Android 10+) SELinux обычно запрещает обычным приложениям доступ к этому файлу `/proc/net/*` / `/sys`. Но политика SELinux настроена **по-разному на разных устройствах и прошивках** (OEM- и кастомные ROM, `permissive`-сборки), поэтому защитой vpnhide следует считать только явно указанное в таблице покрытие.

**libc** = best-effort покрытие Zygisk: прямой системный вызов обходит inline-хук.

**опц.** = скрытие файловых путей в `.ko` по умолчанию выключено. Включите его
в настройках и перезагрузите устройство; в выключенном состоянии глобальные
VFS-хуки вообще не устанавливаются.

Важно: `ioctl` и netlink-дампы доступны обычному приложению без помощи SELinux; на Linux 5.7+ это относится и к первой привязке сокета к интерфейсу. Именно через netlink детекторы вроде RKNHardering обходят блокировку `/proc/net/route` (см. [issue #86](https://github.com/okhsunrog/vpnhide/issues/86)). Kernel-level варианты (kmod/KPM) закрывают отмеченные в таблице нативные пути без следа в процессе. Zygisk закрывает только вызовы, проходящие через libc; прямой raw syscall обходит его хуки. На более старых ядрах привязку непривилегированного сокета отклоняет само ядро. Остальное либо часто блокируется SELinux на стоке (но это зависит от устройства), либо идёт через Java API и покрывается LSPosed.

KPM в бете: он реализует те же 11 логических kernel-хуков, что и `.ko`, но отдельные различия ABI и поведения на старых ядрах перечислены в полной карте векторов.

Полная карта векторов — с разбивкой по слоям, нюансами SELinux и известными пробелами — в [docs/detection-vectors.md](docs/detection-vectors.md).

## Сборка из исходников

- **kmod**: `./kmod/build.py --kmi android14-6.1` (или `--all`) — авто-запускает DDK-контейнер через podman/docker. Подробнее: [kmod/BUILDING.md](kmod/BUILDING.md).
- **KPM**: `python3 kmod/kpm/build.py` — собирает универсальный `vpnhide-kpm.zip` через KernelPatch submodule. Подробнее: [kmod/kpm/README.md](kmod/kpm/README.md).
- **zygisk**: `cd zygisk && ./build.py` (Rust + NDK + cargo-ndk)
- **lsposed**: `cd lsposed && ./gradlew assembleDebug` (JDK 17 + Rust + NDK + cargo-ndk). Обычный debug APK остаётся debuggable, но проходит R8/shrink, чтобы не раздувать Xposed-модуль при установке и agent smoke-тестах. Для старой unminified debug-сборки используйте `./gradlew assembleRawDebug`.

### Заметки для контрибьюторов, застрявших на Windows

Если вы используете Windows, при сборке некоторых подпроектов возникают определенные неудобства.

**lsposed**: отлично собирается в Android Studio.

**portshide**: `cd .\portshide\; python .\build-zip.py` выполняется без проблем.

Для kmod и zygisk вам (к сожалению) потребуется установить [Docker for Windows](https://docs.docker.com/desktop/setup/install/windows-install/).

**kmod**: `python .\kmod\build.py --kmi android14-6.1` — скрипт сам подберёт docker и поднимет образ `ddk-min` (тот же, что в CI).

**KPM**: собирайте из Linux или WSL. Скрипт ожидает POSIX-инструменты, `make`/`clang`, KernelPatch submodule и Android NDK; нативная сборка на Windows сейчас не документирована.

**zygisk**:
```powershell
docker run --rm -it -v "${PWD}:/workspace" -v "vpnhide_cargo_cache:/usr/local/cargo/registry" -w /workspace ghcr.io/okhsunrog/vpnhide/ci:latest bash -c 'cd zygisk && python3 ./build.py'
```
Причина, по которой `zygisk` нельзя собрать напрямую, заключается в том, что исходный код зависимости `zygisk-api` содержит файл с именем `aux.rs`. Cargo использует `libgit2` для работы с git, в котором есть защита, запрещающая создавать файлы, _содержащие_ зарезервированные слова Windows. Вы получите ошибку: `cannot checkout to invalid path 'src/aux.rs'; class=Checkout (20)`. [Сообщают](https://superuser.com/a/1929659), что после какого-то обновления стало возможным создавать файлы, содержащие зарезервированные слова, **с** расширением, но, похоже, в `libgit2` это поведение не было изменено.

## Проверено на

- [RKNHardering](https://github.com/xtclovver/RKNHardering/) — все векторы обнаружения чисты
- [YourVPNDead](https://github.com/loop-uh/yourvpndead) — все векторы обнаружения чисты

Оба реализуют официальную методику обнаружения VPN/прокси Минцифры РФ ([источник](https://t.me/ruitunion/893)).

## Раздельное туннелирование (split tunneling)

Корректно работает с конфигурациями VPN с раздельным туннелированием. Затрагиваются только приложения из списка целей.

Настоятельно рекомендуется использовать split tunneling в паре с VPN Hide.

Приложения-детекторы, сравнивающие публичный IP устройства с внешними чекерами, лучше оставлять вне туннеля — их трафик должен выходить через оператора, а не через VPN.

## Модель угроз

vpnhide скрывает активный VPN от конкретных приложений. Он НЕ предназначен для:
- Скрытия root или кастомной прошивки
- Обхода Play Integrity
- Обмана серверной детекции (утечки DNS, чёрные списки IP, фингерпринтинг латентности/TLS)

## Известные ограничения

- `kmod` требует поддерживаемое GKI-ядро с `CONFIG_KPROBES=y` (стандарт на устройствах Android 12+)
- KPM требует KernelPatch runtime (APatch или KPatch-Next-Module); не ставьте KPM одновременно с `.ko`
- `lsposed` требует LSPosed, LSPosed-Next или Vector
- `zygisk` — только arm64
- Прямые системные вызовы `svc #0` обходят хуки libc в Zygisk — для этого нужен kernel-level backend (kmod или KPM)
- Серверная детекция неисправима на стороне клиента — используйте раздельное туннелирование

## Поддержать проект

vpnhide бесплатный, без рекламы и телеметрии. Платных функций не будет — донат ничего не открывает.

Тот же список есть в приложении: **Настройки → Поддержать проект**, там адреса копируются по тапу.

**Boosty** — оплата картой, разово или подпиской: <https://boosty.to/okhsunrog/donate>

| Монета | Сеть | Адрес |
| --- | --- | --- |
| USDT | Tron (TRC20) | `TMskx2wKmPg11VYvHoS93vUQGm7yhcetUU` |
| BTC | Bitcoin | `bc1pmt9u6nux4x7n86zknwdgt9v02lah2tu6d983ak2prc5cwt8hsetq82ganh` |
| GRAM | The Open Network | `UQADYTtMBQdZvmNNEX02R9sACpdnXKlPV8RbuFrxo7JFBRGS` |
| LTC | Litecoin | `MBLKJfPNANH3U41UPJFtha7EPJGdbiW5dZ` |

## Лицензия

MIT. См. [LICENSE](LICENSE).

Модуль ядра объявляет `MODULE_LICENSE("GPL")`, как требуется ядром Linux для разрешения символов `EXPORT_SYMBOL_GPL` во время выполнения.

## Star History

<a href="https://www.star-history.com/?type=date&repos=okhsunrog%2Fvpnhide">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=okhsunrog/vpnhide&type=date&theme=dark&legend=top-left&sealed_token=L6VLoQFGmusCfaI01irFbE2MJWoOX9V4Z66YMzG6z0vD-xjku8IZX4jnDHFYeAjjEne48AgxfoSExLa90tYlqeYq7E32T0DGbdKrR8UTyp0zVCfDeKGdCIku_20sKVi9WBuO4aqa3nBKDnKepezie3AC67kmr-2mazo76SIUyXWpRp8Lb038KZtafha8" />
    <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=okhsunrog/vpnhide&type=date&legend=top-left&sealed_token=L6VLoQFGmusCfaI01irFbE2MJWoOX9V4Z66YMzG6z0vD-xjku8IZX4jnDHFYeAjjEne48AgxfoSExLa90tYlqeYq7E32T0DGbdKrR8UTyp0zVCfDeKGdCIku_20sKVi9WBuO4aqa3nBKDnKepezie3AC67kmr-2mazo76SIUyXWpRp8Lb038KZtafha8" />
    <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=okhsunrog/vpnhide&type=date&legend=top-left&sealed_token=L6VLoQFGmusCfaI01irFbE2MJWoOX9V4Z66YMzG6z0vD-xjku8IZX4jnDHFYeAjjEne48AgxfoSExLa90tYlqeYq7E32T0DGbdKrR8UTyp0zVCfDeKGdCIku_20sKVi9WBuO4aqa3nBKDnKepezie3AC67kmr-2mazo76SIUyXWpRp8Lb038KZtafha8" />
  </picture>
</a>
