# RFC: Автоматическое определение GKI/KMI и безопасный подбор kmod в VPNHide 2.x

**Идентификатор:** `RFC-AUTO-GKI-KMOD-001`  
**Статус:** Proposed / требует реализации и проверки на устройствах  
**Дата:** 2026-11-02  
**Проект:** [okhsunrog/vpnhide](https://github.com/okhsunrog/vpnhide), ветка [`dev-2.x`](https://github.com/okhsunrog/vpnhide/tree/dev-2.x)  
**Рассмотренный snapshot:** `e50134df6cf513b54eea12add73084af6a730f42`  
**Область:** unified Core, обнаружение GKI/KMI, загрузка `vpnhide_kmod.ko`, journaled native lifecycle, диагностика и UX  
**Назначение:** архитектурный документ для агента-разработчика; **не** означает, что описанная функциональность уже реализована.

---

## 0. Краткое решение

VPNHide 2.x должен **самостоятельно** определить подходящий `vpnhide_kmod.ko` из комплекта Core. Пользователь **не должен** знать, что такое `android13-5.15`, `android14-5.15`, KMI, `vermagic`, либо вручную скачивать/перебирать варианты. В обычном сценарии он устанавливает/обновляет **один Core**, а Core выбирает и активирует подходящую реализацию.

Предлагается многоступенчатый алгоритм:

1. Собрать **пассивные свидетельства** о ядре: Linux major/minor, полный `uname`, при необходимости сведения об установленных и действительно используемых kernel modules (`vermagic`, CRC), точный Build ID (если доступен и распознаваем), и иные диагностические сигналы.
2. По Linux LTS и проверенному manifest сформировать **все допустимые KMI-кандидаты**; например, для 5.15 — `android13-5.15` и `android14-5.15`. Строка `androidXX` из `uname` — свидетельство, **не безусловная истина**.
3. Для неопределённых кандидатов выполнить **bounded loader preflight**: вызвать `finit_module()` для **того же самого финального `.ko`** с особым параметром (`preflight_only=1`, имя предварительное). Модуль должен немедленно завершить `module_init` отрицательным специальным кодом, **не регистрируя хуки, не создавая `/proc`-узлы и не оставляя резидентного экземпляра**. Достижение этого пути означает прохождение предшествующих проверок загрузчика — но не доказательство полной runtime-совместимости ABI.
4. При наличии достаточно сильных свидетельств выбрать **ровно один** бинарник; если оба прошли preflight и определить нужное поколение нельзя — **не угадывать** по порядку. Рассмотреть другой безопасный native backend с учётом всей требуемой политики и текущего ownership; иначе опубликовать типизированную причину невозможности активации.
5. Выполнить **ровно одну полноценную загрузку** выбранного `.ko`, проверить идентичность live backend, обязательные хуки, control protocol и доставку конфигурации. Полноценный `finit_module` не повторять слепо после ошибки/краша.
6. Journal должен до каждого потенциального эффекта фиксировать его точные bytes/hash, boot identity, границу вызова, результат или неопределённость; восстановление после crash/restart не должно запускать второй реальный `.ko` поверх первого.
7. UI публикует итог («Защита активна» / «Всё работает» при достоверном подтверждении) либо понятную проблему. Подробности KMI, CRC, попыток и `errno` — **только** в технической диагностике.

**Критическое ограничение:** Linux module-loader preflight — не чисто пассивное сравнение ABI. Это реальная операция с загрузчиком ядра и исполняемым `module_init`. Она должна сначала пройти host/QEMU/device acceptance. Пока её безопасность не подтверждена, допустим консервативный режим с отказом от неоднозначного выбора.

---

## 1. Цели и требования

### 1.1. Цели

- Полностью исключить ручной выбор и установку одного из нескольких GKI ZIP при использовании unified Core 2.x.
- Уметь работать, когда `uname -r` урезан, повреждён, переопределён кастомным ядром или не содержит `androidXX`.
- Работать с однозначными и неоднозначными Linux LTS, с возможностью будущего расширения каталога KMI.
- Не выбирать несовместимый модуль на основании версии Android-прошивки (`Build.VERSION.RELEASE`, SDK API level) или единственной эвристики.
- Использовать уже существующие **проверенный payload catalog**, **single-writer CommandEngine**, **Prepared/TransitionIntent**, **native journal/hold/quarantine**, **runtime ownership**.
- Разделять *достоверную загрузочную совместимость*, *эмпирическую активацию* и *эффективность защиты*.
- Исключить ситуацию, когда два native backend или два kmod экземпляра конфликтуют.
- Сохранять всю сложность автоматического выбора внутри Core; APK — отображение typed state, а не самостоятельный управляющий загрузкой модулей.

### 1.2. Не-цели

- Доказать полную совместимость всех внутренних структур ядра только по CRC экспортируемых символов.
- Обходить подпись модулей, `CONFIG_MODULE_SIG_FORCE`, SELinux/LSM, lockdown, `modules_disabled` или запреты root-менеджера.
- Использовать `MODULE_INIT_IGNORE_VERMAGIC` или `MODULE_INIT_IGNORE_MODVERSIONS`, принудительную подмену CRC/ELF либо другие опасные обходы защит загрузчика.
- Автоматически патчить/прошивать boot image, менять ядро или перезагружать телефон без отдельного осознанного действия пользователя.
- Принимать наличие payload внутри ZIP за признак, что backend установлен либо активен.
- Считать кэшированный успех полноценной гарантией после OTA, смены ядра или изменившихся backend-конфликтов.
- Реализовать migration 0.x/1.x целиком в этом RFC; при этом его ограничения ownership/obsolete modules должны соблюдаться.

### 1.3. Пользовательский контракт

- Пользователь не выбирает поколения `androidXX` и не скачивает альтернативные KMI ZIP.
- Core автоматически пробует **только обоснованные и безопасные** варианты, в ограниченном количестве.
- При наличии безопасной альтернативы применяет её по общей политике селектора, а не предлагает пользователю разбираться с KMI.
- Если автоматическое исправление невозможно, интерфейс объясняет, что защита не активирована, и предлагает доступные реальные действия («Диагностика», а когда есть определённое исправление — «Починить конфигурацию»). Не выводить ложную кнопку исправления без исполнимого плана.
- В UI не требуется отдельный экран «Выберите GKI», а также сообщения «попробуйте android13, затем android14».

---

## 2. Текущая реализация в `dev-2.x` (проверено по snapshot)

> **Важно:** репозиторий находится в переходном состоянии. Примитивы unified bundle уже существуют, но production-интеграция селектора с lifecycle пока не завершена. Не трактовать документы будущих этапов как уже реализованное поведение.

| Область | Состояние в snapshot | Что требуется |
|---|---|---|
| `lsposed/.../DashboardData.kt` | `buildNativeInstallRecommendation()` парсит `uname -r` и предлагает первичный/альтернативный ZIP для 5.10 и 5.15 | Перенести **факт выбора** в Core; отказаться от UI-выбора бинарника |
| `DashboardData.kt` / `classifyKmodProblem()` | `AmbiguousLoadFailed` предлагает другой вариант даже без доказательства, что ошибка вызвана KMI | Заменить typed diagnostic outcome от Core |
| `crates/core-daemon/src/payloads/mod.rs` | Закрытый перечень 7 KMI, manifest, hash/size validation, безопасное открытие, pinned descriptors | Оставить; использовать как authoritative source доступных бинарников |
| `crates/core-daemon/src/payloads/selection.rs` | `Capabilities.kmi: Option<String>`; `Kmod` допускается только при **точном** совпадении с manifest; `None => UnknownKmi` | Поддержать **упорядоченный список KMI-кандидатов и evidence**, а не только exact string |
| `scripts/payload_bundle.py` | Объединяет 7 kmod ZIP, KPM, Zygisk и Core в единый проверяемый ZIP | Дополнить release/manifest metadata при необходимости; не создавать отдельные пользовательские ZIP |
| `crates/activator/src/load_kmod.rs` | Android вызывает `finit_module(2)`, host-тесты используют `insmod` double | Добавить контролируемый `preflight`, сохранив передаваемый `errno` |
| `crates/core-daemon/src/native_boot.rs` | Один native backend per boot, `load_kmod()` грузит один `.ko`, затем проверяет `/proc/vpnhide_ctl` | Разделить подготовку выбора, preflight и единственный real load |
| `crates/core-daemon/src/native_boot/prepared.rs` | Staging приватного snapshot бинарника, prepared intent, reload/recovery gate | Зафиксировать **несколько candidate payloads** и протокол попыток |
| `crates/core-daemon/src/journal.rs` | Ограничение реальных загрузок по `NativeBackend` и boot, quarantine по sha256, отсутствие слепого replay | Отделить **краткоживущие preflight attempts** от полноценной загрузки |
| `kmod/vpnhide_kmod.c` | `vpnhide_init()` регистрирует kretprobe; `module_exit()` преднамеренно отсутствует | Early-return `preflight_only` **до первого побочного эффекта VPNHide** |
| `docs/notes/unified-core-implementation.md` | Фиксирует границы этапов: каталог/журнал/сборка реализованы; production selection/fallback ещё не подключены | Интегрировать RFC в следующий этап, не обойти guards |

Поддерживаемый в текущем bundle список:

```text
android12-5.10
android13-5.10
android13-5.15
android14-5.15
android14-6.1
android15-6.6
android16-6.12
```

`android17-6.18` (если будет нужен) не должен предлагаться, пока **проверенный `.ko` и manifest** для него не включены в сборку.

### 2.1. Важная деталь о текущем kmod

Конец `vpnhide_kmod.c`:

- вызывает `module_init(vpnhide_init)`;
- **не** содержит `module_exit` (постоянная резидентность до reboot);
- после регистрации hook-ов имеет активные kernel-visible эффекты.

Поэтому **перебор полноценных `.ko` посредством real load, затем `rmmod`, затем следующий** — неприемлемая основная стратегия. Первый успешно загруженный экземпляр не снимается обычным `rmmod`, а сбой `module_init` может произойти *после* частичных эффектов.

---

## 3. Термины и модель доверия

- **Linux series/LTS** — например `5.15`, `6.1`; *ограничивает* множество кандидатов, но не всегда определяет KMI.
- **GKI branch / KMI family** — `android14-5.15`, `android14-6.1`, ...; не равна версии установленной Android ОС.
- **KMI generation** — более узкая спецификация вида `5.15-android14-N`; текущий манифест VPNHide адресует семейства `android14-5.15`, не все возможные поколения и модификации.
- **Release hint** — `uname -r`; может быть неполным или неверным. `uname -v`/boot properties тоже только дополнительные сигналы.
- **Vermagic** — метаданные собранного модуля; если взяты из модуля, который действительно загружен, они полезнее, чем из неиспользуемого файла на диске.
- **Module CRC / `CONFIG_MODVERSIONS`** — проверка ABI для *использованных экспортируемых символов*. Не охватывает произвольные private structures, скрытые символы и семантику hook-ов.
- **Loader preflight** — операция, при которой тот же модуль доходит до специального раннего `module_init`, возвращает отрицательный sentinel и **не оставляет VPNHide hooks**. Это **не** `insmod -n` и **не** отсутствие исполнения кода ядра.
- **Full activation** — реальная загрузка без preflight, регистрация хуков и подтверждение control-protocol и политики.
- **Unknown / ambiguous outcome** — результат нельзя доказать; не считать его успешным и не выполнять неконтролируемый retry.

Градации evidence:

| Источник | Назначение | Чего не гарантирует |
|---|---|---|
| Linux major/minor | Ограничить кандидатов | Отличить `android12/13` на 5.10 или `android13/14` на 5.15 |
| `uname` с `androidXX` | Упорядочить кандидатов, иногда однозначно выбрать для типовых GKI | Честность строки кастомного ядра |
| `vermagic` vendor `.ko` | Уточнить семейство/ветку | Что модуль действительно применим к **запущенному** ядру; отсутствие кастомного `KERNELRELEASE` |
| CRC импортированных символов vendor `.ko` | Дополнительный ABI fingerprint | Уникальность fingerprint или полноту покрытия приватных структур |
| Build ID работающего ядра | Точное сопоставление с **известной** эталонной сборкой | Соответствие кастомного ядра официальному Build ID |
| Loader preflight | Проверки загрузчика плюс достижение раннего `module_init` | Полную работоспособность хуков и корректность private layouts |
| Full-load + control handshake | Резидента и нужную версию интерфейса | Что все необходимые detection vectors реально скрыты |
| Runtime diagnostics | Подтверждение покрытия/результата защиты | Работа на всех будущих конфигурациях приложений |

**Принцип:** «сработал `finit_module`» не эквивалентно «правильное поколение KMI»; «вызов достиг preflight» тоже не эквивалентно «все хуки безопасны».

---

## 4. Пассивная идентификация ядра

### 4.1. Источники и порядок

1. `uname(2)`: `release`, при необходимости `version`, `machine`; разбирать **ведущие** `major.minor`, с проверкой границ, а не неограниченным поиском версии посреди произвольного текста.
2. Найти кандидатов только по имеющимся в **verified manifest** `KmiId` и соответствующей Linux series.
3. Если `androidXX-<major>.<minor>` прочитан корректно, считать его **сильным hint**; при сомнениях проверять через preflight/другие сигналы.
4. При неоднозначности и наличии разрешённых файлов vendor/system modules исследовать `.modinfo/vermagic` — предпочтительно для **действительно загруженных** модулей (сопоставление имён в `/proc/modules`/`/sys/module`), с учётом доступности данных.
5. При необходимости исследовать пары `(symbol, CRC)` из секций версий модулей; сопоставлять с эталонами для **конкретных** KMI. Не делать решение по одному совпадающему CRC.
6. Build ID (`/sys/kernel/notes`, если доступен) сравнивать только с **достоверной таблицей точных сборок**. Не выводить KMI из произвольного неизвестного ID.
7. VINTF/свойства Android и boot metadata допускаются лишь как низкоприоритетная помощь/диагностика, не как самостоятельный критерий binary compatibility.

Парсер `.ko` желательно сделать в Rust с ограничениями на размер, ELF-секции, количество файлов и время; не полагаться на присутствие `modinfo` как внешней команды. Ни один непроверенный путь из файловой системы не должен автоматически становиться кандидатом на загрузку.

### 4.2. Matrix для текущего bundle

| Linux series | Кандидаты из текущего bundle | При отсутствии `androidXX` |
|---|---|---|
| `5.10` | `android12-5.10`, `android13-5.10` | Неоднозначность; уточнять автоматически |
| `5.15` | `android13-5.15`, `android14-5.15` | Неоднозначность; уточнять автоматически |
| `6.1` | `android14-6.1` | Один кандидат, но **не доказательство runtime ABI** |
| `6.6` | `android15-6.6` | Один кандидат |
| `6.12` | `android16-6.12` | Один кандидат |
| Иное | Нет | Не притворяться, что есть совместимый kmod |

**Не делать:** выбирать android14 по `Build.VERSION.SDK_INT=34/35/36`, полагаться только на `uname` без проверки реальных ограничений загрузчика, считать любое несовпадение `vermagic` доказательством неправильной ветки KMI.

### 4.3. Сохранение результата

В debug evidence сохранять:

- `linux_series`, исходные `uname.release`/`uname.version`, если доступны;
- список KMI-кандидатов и какой сигнал их добавил;
- `reported_kmi` отдельно от `resolved_kmi`;
- источники/степень уверенности (`exact_build_match`, `version_hint`, `vendor_module_vermagic`, `crc_fingerprint`, `loader_preflight`, `empirical_live`);
- флаг `ambiguous` и **почему** выбор остановлен.

Использовать эти поля также для инвалидации кэша при изменении ядра или bundle.

---

## 5. Loader preflight с тем же `.ko`: рекомендуемый эксперимент

### 5.1. Почему не полноценная пробная загрузка

В VPNHide `.ko` не предусматривает выгрузку после `module_init` success. Попытка «загрузить один, проверить, выгрузить, загрузить другой» нарушает этот инвариант. Кроме того, реальный `module_init` регистрирует hooks, а ошибка позднее не доказывает отсутствие побочных эффектов.

### 5.2. Предлагаемое изменение C

Добавить специальный **неэкспортируемый в UI** параметр, например `preflight_only`; термин предпочтительнее `abi_probe`, поскольку проверяет **прохождение loader checks**, а не весь ABI.

Упрощённый иллюстративный фрагмент (**перед реализацией сравнить с текущим деревом и собрать всеми DDK**):

```c
static bool preflight_only;
module_param(preflight_only, bool, 0);
MODULE_PARM_DESC(preflight_only,
                 "VPNHide loader preflight; do not activate hooks");

static int __init vpnhide_init(void)
{
    /* ДОЛЖНО быть до любых VPNHide side effects. */
    if (unlikely(preflight_only))
        return -EUCLEAN;  /* Предварительно выбранный sentinel. */

    /* Существующий production init без функциональных изменений. */
    /* ... register_kretprobe(), proc_create(), etc ... */
}
```

**Инварианты:**

- До этого `if` не выполнять `register_kprobe`/`register_kretprobe`, `proc_create`, конфигурационные записи, создание worker/thread, выделение долгоживущих VPNHide resources и другие runtime side effects VPNHide.
- Не возвращать `0` в preflight: это сделало бы `.ko` успешно загруженным и **невыпускаемым до reboot**.
- Не добавлять `module_exit()` исключительно ради проверки.
- `-EUCLEAN` — *предложение*, а не стандартизованный код preflight. Выбрать/задокументировать один код; предусмотреть тест, что конкретные GKI доставляют отрицательный init errno обратно userspace без преобразования.
- Не считать `errno == EUCLEAN` криптографическим доказательством достижения init: теоретически внешняя часть загрузчика или OEM-патч тоже может вернуть этот errno. Поэтому нужен доверенный заверенный payload, контролируемый init-path, проверка постусловий и device gate.
- Linux loader может выполнить собственные операции **до** `module_init`, в том числе kernel module constructor/notifier paths в зависимости от версии/config. Preflight **не полностью пассивен** и не гарантирует отсутствия любых kernel-level побочных эффектов. Если конкретная сборка имеет constructors, убедиться, что в них нет VPNHide side effects.

### 5.3. Userspace-вызов

```rust
// Иллюстративный интерфейс, не готовый patch.
enum KmodProbeResult {
    ReachedInitSentinel,
    RejectedBeforeInit { errno: i32, category: LoadRejection },
    Indeterminate { errno: Option<i32>, detail: String },
}

fn preflight_kmod(pinned_fd: &std::fs::File) -> KmodProbeResult {
    // finit_module(pinned_fd, "preflight_only=1", 0)
    // Конкретную errno-классификацию реализовать отдельно.
    todo!()
}
```

Результат `finit_module` с **плановым `errno=EUCLEAN`** трактуется как `ReachedInitSentinel`, **не** как ошибка активации и **не** как настоящий успешный `insmod`. Успешный `Ok(())` в режиме preflight — **инвариант нарушен**: модуль мог остаться резидентным; немедленный HOLD, никакого следующего кандидата.

Проверять постусловия после *каждой* попытки:

- Наш `/proc/vpnhide_ctl` не возник или не изменил владельца, нет live VPNHide kmod, не появились собственные hooks/процессы;
- отсутствие внешнего conflict/owner change до и после;
- модуль с именем VPNHide не остался загруженным (где этот факт наблюдаем);
- таймаут/пропавший ответ **не** равен доказанному `RejectedBeforeInit`.

### 5.4. Когда разрешён следующий preflight-кандидат

Разрешён **только** если предыдущая попытка завершилась достоверно, собственные побочные эффекты VPNHide отсутствуют, kernel ownership остаётся clear и нет неизвестных outcomes.

- `ReachedInitSentinel`, модуль не живёт → допустим следующий **preflight**, если нужно проверить неоднозначность.
- Документированный отказ загрузчика **до** `module_init` (например, доказанный symbol/version rejection), runtime clean → следующий preflight допустим.
- `EPERM`, `EACCES`, `EKEYREJECTED`, `ENOKEY`, disabled modules, SELinux отказ, сбой доступа к FD и другие **глобальные** ограничения загрузчика → **не** перебирать другие KMI; это не решается выбором androidXX.
- `EINVAL` без достаточной диагностики (включая неизвестный параметр), `EINTR`/timeout, отказ неизвестного происхождения, `Ok(())` вместо sentinel, crash/kill workers → **не** пробовать другой бинарник автоматически; нужно reconcile/hold.
- Никакого `delete_module` как обязательного этапа: корректный отрицательный init не делает модуль постоянно загруженным.

### 5.5. Важные ограничения preflight

1. `CONFIG_MODVERSIONS` проверяет только **импортированные экспортируемые** символы: два GKI могут пройти одинаково.
2. Отсутствие `CONFIG_MODVERSIONS` или нарушение KMI у vendor kernel ограничивает информативность успешного preflight.
3. `vermagic` может быть неприменим как идентификатор ветки при `CONFIG_MODVERSIONS` (часть release может игнорироваться при наличии CRC), но остальные флаги ядро продолжает учитывать.
4. `pr_info`/`dmesg` может помочь в диагностике, но текст kernel log, доступ и смешение сообщений от разных попыток **не должны** быть единственным источником принятия решения.
5. Preflight не проверяет layout приватных структур, CFI/LTO-особенности kretprobe callbacks и реальную скрывающую функциональность.
6. Если реальное ядро небезопасно реагирует на сам loader preflight, feature **не включать** для этого класса устройств — нужен безопасный отказ или альтернативный backend.

---

## 6. Decision algorithm и политика выбора

### 6.1. Предлагаемая чистая модель

Вместо одного `Capabilities.kmi: Option<String>`:

```rust
struct KernelIdentity {
    linux_series: Option<LinuxSeries>,
    reported_kmi: Option<KmiId>,
    build_id: Option<BuildId>,
    evidence: Vec<KernelEvidence>,
}

struct KmiCandidate {
    kmi: KmiId,
    payload_index: usize,  // Только из проверенного Catalog.
    rationale: Vec<EvidenceKind>,
}

enum KmiResolution {
    NoCandidate { reason: NoCandidateReason },
    Unique { candidate: KmiCandidate, evidence: Vec<KernelEvidence> },
    Ambiguous { candidates: Vec<KmiCandidate> },
}

// Далее — отдельный эффектный этап, не часть pure selector.
enum ProbeVerdict {
    EligibleByLoader,
    Rejected { category: LoadRejection, errno: Option<i32> },
    Unknown { detail: String },
}
```

Точные Rust-типы адаптировать к текущему `payloads::selection::Assessment`, не создавать параллельную независимую систему выбора.

**Важно:** наличие кандидата **не** означает native eligibility. Сохраняются проверки архитектуры, runtime availability, ownership, policy resolution, hook coverage/capacity, wire capacity, default mask и аутентификации иных backend.

### 6.2. Правила принятия решения

1. **Preflight выполняется только для kmod** и только если весь native lifecycle даёт право проверить этот backend. Наличие внешнего native owner блокирует попытки.
2. Если 0 подходящих KMI-кандидатов, kmod неподходящ; перейти к оценке иных backend.
3. Если 1 кандидат, проверить его известные факты/возможность загрузчика. Один кандидат сам по себе не доказывает runtime safety; при включённом feature gate выполнить preflight (особенно если `uname` сомнителен).
4. Если 2 кандидата, собрать пассивные evidence и выполнить preflight в **детерминированном** порядке. Не прекращать сбор доказательств при первом `ReachedInitSentinel`, если второй может тоже пройти.
5. Если ровно один `EligibleByLoader`, его можно выбрать **при отсутствии противоречащих сильных свидетельств** и с явным ограничением, что это всё ещё не 100%-ный ABI proof.
6. Если несколько `EligibleByLoader`, выбрать только при дополнительном **надёжном** разграничивающем evidence (например, эталонный Build ID известной сборки; надёжная комбинация CRC/branch identity). **Не** выбирать просто первый по хронологии или `uname` со спорным происхождением.
7. Если все отвергнуты — kmod unavailable; другой backend оценивается общим selector.
8. Если хотя бы одна попытка **unknown** или появился residual owner — сохранить HOLD. Не выполнять автоматический переход к другой реализации, способный привести к конфликту хуков.
9. Если другой backend выбран, проверить **всю требуемую policy**, наличие реального runtime и его безопасное состояние; не объявлять успешной заменой backend с меньшим необходимым покрытием.
10. При успешном production `finit_module` проверить control handshake/версию, зарегистрированные обязательные хуки и доставку конфигурации. Только после этого Core может публиковать `selected/applied/healthy` по соответствующим отдельным признакам.

### 6.3. Предлагаемая последовательность

```text
Boot Core
  |
  +-- Load/reconcile durable journal and running native ownership
  |      \-- unknown/conflict -> HOLD + diagnostics
  |
  +-- Open verified Catalog (active generation only)
  |      \-- corrupt/missing -> typed failure, no unsafe load
  |
  +-- Pure capability/policy assessment
  |
  +-- Passive kernel evidence -> bounded KMI candidates
  |
  +-- [kmod eligible?]
  |      |
  |      +-- no -> assess other allowed native backends
  |      |
  |      +-- yes -> prepare and journal exact candidate list/hashes
  |                    |
  |                    +-- preflight each necessary candidate
  |                    |     \-- any unknown/residual -> HOLD
  |                    |
  |                    +-- resolve unique safe candidate?
  |                           |
  |                           +-- no -> other safe backend / typed failure
  |                           |
  |                           +-- yes -> journal real-load intent
  |                                      |
  |                                      +-- exactly one real finit_module()
  |                                             |
  |                                             +-- unknown -> HOLD
  |                                             +-- failed known -> typed failure;
  |                                             |                  no blind real-load retry
  |                                             +-- live -> verify + configure
  |
  +-- Publish structured status to APK
```

---

## 7. Journal, crash consistency и изоляция эффектов

### 7.1. Почему существующий `NativeLoadBudget` недостаточен

Сейчас `Journal::check_native_attempt()` ограничивает попытки на уровне `NativeBackend::Kmod` за один boot. Для нового алгоритма необходимы **несколько проверок разных payloads** без предоставления права на несколько real loads.

**Не стоит** просто расширять лимит попыток `Kmod` с одной до двух: это разрешит и второй полноценный `finit_module`, что нарушает инвариант.

### 7.2. Разделить бюджеты

- `PreflightBudget { boot_id, attempted_payload_hashes, count, resolution }` — только строго ограниченные загрузки в `preflight_only` с доказанным cleanup; максимум число подходящих кандидатов текущей Linux series, без произвольных повторов.
- `RealLoadBudget { boot_id, attempted_backend, payload_sha256, state }` — **не больше одного dispatched real load** на boot, пока нет подтверждённого безопасного алгоритма восстановления; после unknown/crash запрещён повтор.
- `Quarantine` — failed/unknown payload + причина + kernel identity; не снимается простым обнулением истории попыток.

Возможность повторять preflight после worker restart зависит от **сохранённого доказательства завершённости** предыдущей попытки, а не от одного признака отсутствия `/proc`.

### 7.3. Подготовка и pinning

`Prepared` должен подготовить весь короткий список кандидатов из verified Catalog, удерживать **конкретные открытые descriptors** и фиксировать их hash/size/KMI. Не разрешать позднему изменению пути или содержимого redirect уже принятого плана.

Ограничения:

- активный модуль Core (`/data/adb/modules/vpnhide_core/...`), не `modules_update`;
- no symlink following, bounded files, hash check до эффектов;
- один manifest generation/bundle commit для всех кандидатов;
- все попытки привязаны к boot ID и точному списку payload identities;
- настоящий `finit_module` должен получить **тот же byte identity**, который прошёл preflight;
- способ предоставления FD (напрямую или через retained private FD path) проверяется на Android SELinux/LSM: host-tests недостаточно.

### 7.4. Долговременная state machine

Предлагаемые переходы, имена условные:

```text
Observed
  -> PreparedCandidates
  -> PreflightPending(candidate)
  -> PreflightResolved(candidate: accepted/rejected)
  -> [next candidate | ResolvedKmi | NoSafeCandidate | HoldUnknown]
  -> RealLoadPrepared(candidate_hash)
  -> RealLoadDispatched
  -> [LiveConfirmed | RealLoadFailedKnown | HoldUnknown]
  -> PolicyApplied
```

Каждое состояние с потенциальными kernel effects должно быть **durably recorded before dispatch** (по правилам существующего CommandEngine). Сохранённый receipt после сбоя не заменяет текущую runtime-проверку.

При crash в окне `PreflightPending` считать preflight **неизвестным** до безопасного reconciliation. При crash в окне `RealLoadDispatched` — **запретить вторую реальную загрузку**, даже если `/proc/vpnhide_ctl` не обнаружен. Возможен adopt только подтверждённого совместимого live owner через уже существующий recovery path.

Предлагается расширить persisted schema с миграцией прежнего состояния (journal schema 4 на исследованном snapshot). Не стирать/игнорировать старые holds и quarantine ради новой функции. `native_boot.json` остаётся диагностическим output, **не** источником authority.

### 7.5. Сосуществование backend

- Builtin имеет фактический kernel ownership и не требует `.ko`; при нём preflight kmod не запускать.
- Если существует live KPM, потенциально live скрытый KPM или foreign kernel owner, preflight не запускать до разрешения конфликта.
- Zygisk authorization и process-local residency учитываются согласно lifecycle Stage 4; нельзя считать простое удаление файла «мгновенной выгрузкой» хуков из существующих процессов.
- Два варианта kmod — **не два native backend**, а кандидаты **одного** backend. Они не должны порождать ложный `NativeBackendsConflict` только потому, что оба бинарника лежат в ZIP.

---

## 8. Диагностика, RPC и UI

### 8.1. Новый typed report от Core

Рекомендуемый набор фактов (поля/имена примерные):

```json
{
  "kernel": {
    "linux_series": "5.15",
    "reported_kmi": null,
    "resolved_kmi": "android14-5.15",
    "resolution": "loader_preflight_with_supporting_evidence"
  },
  "kmod_selection": {
    "status": "selected",
    "selected_payload_sha256": "<sha256>",
    "attempts": [
      { "kmi": "android13-5.15", "phase": "preflight", "result": "rejected_by_loader", "errno": 8 },
      { "kmi": "android14-5.15", "phase": "preflight", "result": "reached_init_sentinel", "errno": 117 }
    ],
    "real_load": "live_confirmed"
  }
}
```

**Это пример схемы и гипотетических результатов, а не реальные данные устройства.** `errno=8`/`117` приведены только для иллюстрации; не прибивать бизнес-логику к неуниверсальному тексту сообщения и не считать `8` единственным кодом ABI rejection.

Нужны отдельные типизированные причины: `no_candidate`, `preflight_loader_rejected`, `all_candidates_rejected`, `ambiguous_multiple_accepted`, `signature_or_permission_denied`, `preflight_outcome_unknown`, `real_load_outcome_unknown`, `live_conflict`, `runtime_hooks_incomplete`, `payload_invalid`. Длительные массивы логов — в debug export, не во все короткие responses.

Если меняется RPC DTO/JSON schema, сделать **согласованный contract bump** в Core, Android-парсере и тестах; не менять смысл старых полей молча.

### 8.2. UI

- Обычный путь: **никакого выбора KMI**, подтверждённое состояние «Защита активна» / «Всё работает».
- Во время разрешения: нейтральное «Подготовка защиты»/«Настройка защиты» только если это действительно видно пользователю; не перегружать стартовый экран низкоуровневыми деталями.
- Если возникла ошибка, **не** показывать «попробуйте другой GKI ZIP» или `androidXX`. Если есть детерминированный recovery plan — «Починить конфигурацию». Если подходящего исправления нет — «Посмотреть диагностику», «Сообщить о проблеме» и понятный статус.
- Различать `loading`, `selected`, `applied`, `measured protection`, а не делать зелёный экран из одного успешного `finit_module`.
- Содержимое подробной диагностики должно быть пригодно для экспорта разработчику (включая kernel identity, список кандидатов, errno, payload hashes и boot-scoped outcomes), без лишних секретов.

### 8.3. Удалить устаревший пользовательский workaround

После production cutover удалить либо перевести в legacy-only следующие сценарии из UI/документации:

- `variantAmbiguous + alternativeArtifact` как предложение пользователю скачать другой ZIP;
- `KmodProblemKind.AmbiguousLoadFailed` как основание «попробуйте запасной модуль»;
- `WrongVariant` как типовая ошибка, которую пользователь должен лечить подбором ZIP при **едином Core**;
- фразы из `docs/help/*/kmod-install.md`, предполагающие самостоятельный выбор/переустановку GKI-модулей.

Во время миграции от 1.x наличие старых модулей и неверных legacy ZIP — отдельная диагностируемая ситуация, но исправление должно проходить через общий Maintenance Plan («Починить конфигурацию»), а не становиться основным UX 2.x.

---

## 9. Ошибки и политика fallback

| Событие | Следующее действие |
|---|---|
| Exact KMI hint отсутствует; LTS имеет двух кандидатов | Пассивное уточнение + разрешённый preflight обоих |
| Один preflight дошёл до sentinel, второй достоверно rejected | Можно выбрать первый, если нет противоречащих фактов; затем **один** real load |
| Оба прошли preflight | Нужны дополнительные надёжные evidence; иначе не угадывать |
| Ни один не прошёл из-за CRC/symbol/version rejection | Kmod отклонён; оценить другой backend |
| `EKEYREJECTED`/`ENOKEY`, modules disabled, permission/SELinux failure | Не перебирать KMI; типизированный глобальный loader blocker |
| Preflight вернул `Ok(())` | Инвариант нарушен; HOLD, дополнительный `finit_module` запрещён |
| Preflight неизвестен / crash / внезапное появление owner | HOLD до safe reconciliation; не продолжать автоматически |
| Real load не удался | Не считать kernel clean по одному errno; без гарантии cleanup **нет** real-load retry другого `.ko` в том же boot |
| Real load прошёл, но нет ожидаемого control node / часть обязательных hooks отсутствует | Не объявлять успех; записать details, не загружать второй kmod поверх первого |
| Kmod недоступен, другой backend подходит по runtime/policy/ownership | Допустим автоматический выбор через общий selector, **только** если safe transition доказан |
| Любой backend активен, но измерения защиты не проходят | Не показывать «Всё работает», диагностировать конкретный пробел |

`signature_rejected()` из `native_boot.rs` следует улучшить: строка `module verification failed` из общего `dmesg` **не обязательно** свидетельствует о запрете загрузки — ядро может лишь отметить taint. Использовать `errno` и атрибуцию сообщений к конкретной попытке; не предлагать другой KMI на основании одной неудачной загрузки.

---

## 10. Альтернативы и обоснование решения

### A. Только `uname -r` + LTS table

**Плюсы:** простота, пассивность. **Минусы:** не различает 5.10/5.15 при отсутствующем Android-теге; version string можно изменить; не подтверждает loader acceptance. Использовать лишь как первый этап.

### B. Чтение `vermagic`/CRC vendor `.ko`

**Плюсы:** пассивно, без изменения ядра; в некоторых случаях уверенно различает ветки. **Минусы:** модулей может не быть, они могут быть не загружены или иметь неверную release string; CRC могут совпадать. Использовать как приоритетный источник дополнительных evidence, не как обязательную предпосылку.

### C. Последовательный **полноценный** `finit_module()` каждого кандидата

**Отклонено:** успешный VPNHide `.ko` нельзя штатно выгрузить до reboot; ошибка второго/первого реального init не доказывает отсутствие частичных hooks и потенциальных проблем. Слишком опасно для автоматического массового UX.

### D. Отдельный минимальный probe `.ko`

**Отложено:** хорошо изолирует собственную инициализацию, но probe проверяет **свой** набор экспортируемых символов/CRC, а не в точности финальный `vpnhide_kmod.ko`. Можно рассмотреть, если preflight внутри финального модуля не проходит device gates.

### E. Специальный preflight в **том же** финальном `.ko`

**Рекомендуемый эксперимент:** та же ELF-идентичность, те же import CRC и основные loader checks; ранний отрицательный init **не оставляет VPNHide hooks**. Но это активная kernel-loader операция, не полный ABI proof и зависит от поведения OEM kernel. Включать только после валидации.

### F. Offline ABI fingerprint + эталонные `Module.symvers`

**Возможное последующее усиление:** более точная идентификация до загрузки, особенно если оба кандидата проходят preflight. Стоимость: хранение/обслуживание эталонов для веток/поколений, неполная информация на OEM kernels, необходимость подобрать информативные символы. Полезное дополнение, не замена live checks.

### G. Попросить пользователя выбрать GKI и вручную прошить второй ZIP

**Отклонено как UX 2.x.** Ручное решение может оставаться только внепродуктовой developer/debug процедурой, не пользовательским способом работы VPNHide.

---

## 11. План реализации по этапам

### Этап 0. Базовые инварианты и inventory

- Зафиксировать и проверить текущую границу между `payloads::selection`, `native_boot`, `Prepared`, `Journal` и APK.
- Ввести единственный источник списка доступных KMI (желательно metadata manifest + общий generated/build-time catalog); устранить дублирующиеся ручные таблицы в Kotlin, Python и Rust, насколько позволяет build pipeline.
- Добавить в Core типизированные `KernelIdentity`, `KmiCandidate` и чистую функцию составления ограниченного списка.
- Тесты на `5.10`, `5.15`, `6.1`, `6.6`, `6.12`, неверный/пустой `uname`, spoofed Android OS version, чужой/отсутствующий payload.

### Этап 1. Пассивные evidence

- Добавить опциональный bounded reader `.modinfo/vermagic` и сведения о загрузке vendor `.ko`; извлечение CRC — при доказанной пользе.
- Отделить `reported_kmi` от `resolved_kmi` и запретить считать низкокачественный hint доказательством.
- Не вводить эффектных операций для сбора этих данных.

### Этап 2. C preflight + unit/QEMU validation

- Изменить `vpnhide_kmod.c`: `preflight_only` как **самая первая** ветка в `vpnhide_init`.
- Добавить `finit_module` params support в `activator` и структурированную classification, сохранив реальный `errno`.
- Проверить отрицательный sentinel / kernel cleanup / отсутствие proc/control/hook side effects на representative GKI.
- Проверить, что `.ko` с `preflight_only` собирается идентично обычному `.ko` по импортам и modversion секциям (не отдельный пробный бинарник).
- **Feature gate для production** до реальных device acceptance tests.

### Этап 3. Journaled candidate selection

- Обновить `payloads::selection::assess()` для списка кандидатов/evidence и поэтапного resolution, сохраняя current policy/ownership guards.
- Расширить `Prepared` для pinning нескольких verified artifacts.
- Добавить отдельный `PreflightBudget` и real-load budget, переходы и durable outcomes; journal migration без потери holds.
- Пройти crash-injection в каждой boundary до/после syscall, до/после journal fsync, между preflight candidates и перед real load.

### Этап 4. Production activation + fallback

- Встроить preflight и single real activation в единый CommandEngine path, а не запускать из UI, root shell или старых модульных скриптов.
- Убедиться, что весь Zygisk/KPM fallback соблюдает ownership, lifecycle, process authorization и полную policy eligibility.
- Устранить ложные дубли конфликтов: две bundled KMI-версии — не два установленных native backend.
- Проверить `ConfigPhase::Native` и receipt semantics, включая unknown/failedKnown.

### Этап 5. Typed RPC / diagnostics / UI

- Опубликовать structured KMI resolution и preflight result; согласованный bump протокола.
- Удалить пользовательские предложения сменить GKI ZIP, заменить на общий статус/типизированную диагностику и исполнимое автоматическое восстановление.
- Обновить help/README/release notes для единого Core.

### Этап 6. Acceptance and rollout

- Провести Android device acceptance на нескольких менеджерах и поколениях GKI.
- Сначала opt-in developer/CI cohort, затем постепенное включение для пользователей при наличии telemetry без чувствительных данных (если проект предусматривает такой канал; не добавлять скрытую аналитику).
- Сохранить fail-closed behavior для неподтверждённых OEM сочетаний.

---

## 12. Тестовая матрица и acceptance criteria

### 12.1. Unit/host

- `uname` точный, пустой, без `androidXX`, с неверной веткой, неправильный `major.minor`, `6.1` vs `6.10`.
- Предметный каталог содержит ровно подходящие кандидаты, не допускает смешения поколения, неизвестного пути, невалидного hash, подмены symlink и staged generation.
- Однозначный и неоднозначный resolution, несколько preflight successes, ни одного success, conflict, unknown, runtime blocker, fallbacks.
- `EUCLEAN`-sentinel, неожиданное `Ok(())`, ошибки подписи, version/symbol mismatch, `ENOMEM`, `EINVAL`, `EPERM`, `EINTR`, отказ доступа, crash.
- Не повторяется real load после неопределённого outcome; не «ослабляется» backend count budget.
- Correct crash/restart/reboot reconciliation, старые journal schema, retention/quarantine, no unsafe replay.
- UI с Core typed status не предлагает пользователю иной KMI ZIP.

### 12.2. Kernel/QEMU

- Минимум две ветки одной Linux series, например обе 5.15, с **реальными** соответствующими `.ko`.
- `preflight_only=1` доходит до sentinel на совместимом ядре, до отказа загрузчика на заведомо несовместимом (если реально есть различие CRC).
- После preflight нет `vpnhide` в `/proc/modules`, нет `/proc/vpnhide_ctl`, не зарегистрированы VPNHide hooks.
- Тот же preflight binary проходит обычную загрузку и дальнейший контроль только после выбора.
- Проверить поведение `CONFIG_MODVERSIONS=y/n`, `CONFIG_MODULE_UNLOAD=n`, module signing, `CONFIG_MODULES=n` или loader disabled, LTO/CFI при наличии доступных target kernels.
- Проверить случай, когда **оба** варианта проходят preflight: система не выбирает первый автоматически.

### 12.3. Реальные Android-устройства

- Magisk, KernelSU/KernelSU-Next, APatch с проверкой полномочий, SELinux, init timing, `O_TMPFILE`/pinned FD, root-manager restart.
- Кастомное ядро с усечённым `uname -r`, две KMI-ветки 5.10 и 5.15, не-GKI kernel.
- Корректная работа после OTA/смены slot и обновления Core.
- Одноразовая реальная загрузка, корректная настройка `filesystem_hiding`, отсутствие kernel owner conflicts.
- При отключении одного backend и замене его другим — process-aware семантика Zygisk/KPM и отсутствующие legacy hooks.
- Не объявлять «Защита активна», если нет подтверждённой актуальной доставки политики или нужного результата диагностики.

### 12.4. Критерии готовности

**Можно включать по умолчанию только если:**

1. Preflight доказуемо не оставляет VPNHide kernel hooks/контрольные узлы на проверенных поддерживаемых kernels.
2. Любой crash/unknown/частичная активация приводит к safe hold без последующего real-load retry в тот же boot.
3. Несколько подходящих кандидатов не разрешаются произвольно; известные причины отказа не превращаются в ложные ошибки KMI.
4. Kmod selection не обходит manifest verification, policy eligibility, ownership, journal, recovery, SELinux и manager boundaries.
5. APK и документация реализуют **один Core без ручного выбора GKI**.
6. Реальный full-load путь прошёл device acceptance; результаты host/QEMU не выдаются за него.

---

## 13. Открытые вопросы, требующие отдельных решений

1. **Какой точный отрицательный errno использовать как sentinel?** Предложен `-EUCLEAN`; проверить переносимость/поведение всех целевых GKI и что нет реального loader path, возвращающего тот же код в конкретной конфигурации.
2. **Как уверенно различать несколько кандидатов, прошедших preflight?** Возможны точный Build ID, подтверждённая ветка из vendor `.ko`, более информативный CRC fingerprint, или безопасный fallback. **Первый в списке — не ответ.**
3. **Нужен ли preflight для однозначного LTS (`6.1`, `6.6`, `6.12`)?** Предлагается да, если feature включён: один вариант в каталоге не доказывает совместимость OEM-ядра. Но при сильной доверенной идентификации это может быть конфигурируемым оптимизационным решением.
4. **Как на всех целевых GKI и Android SELinux работает `finit_module` от pinned anonymous FD?** На host path сейчас уже есть private staging; нужен device test.
5. **Как сериализовать кандидатов без ломки existing Journal и чрезмерного расхода flash?** Хранить hash/identity и короткий outcome, не копии всех бинарников в журнал.
6. **Можно ли безопасно запускать preflight до завершения всей KPM/Zygisk inventory?** По умолчанию нет: сначала owner/conflict gate.
7. **Какую policy допускается автоматически применить альтернативному backend?** Только такую, для которой доказано покрытие всех требуемых возможностей. Не подменять «нет подходящего kmod» на «что-нибудь работает».
8. **Как избежать ложных выводов из общего `dmesg`?** Собирать кросс-проверенные kernel log фрагменты по времени и boot identity для диагностики; authoritative outcome — syscall + собственные observations/journal.
9. **Как быть с модифицированными OEM/private hook layouts?** Даже успешная загрузка/CRC не доказывает безопасность всех `pt_regs` и структур; требуется runtime hook validation и, возможно, явная политика отключения рискованных hooks.
10. **Как поддерживать новые ветки GKI?** Только через версионированный artifact catalog, CI и device acceptance; отсутствие payload не компенсировать guessing.

---

## 14. Конкретные указания агенту-разработчику

1. **Не** начинать с переписывания Kotlin-таблицы на «автоматически пробуй второй ZIP». Выбор должен жить в unified Core runtime, а Kotlin — получать typed result.
2. **Не** делать два полноценных `insmod` подряд. Preflight и real activation — различные операции, различные журналируемые бюджеты и разные постусловия.
3. **Не** стирать существующий journal и не обходить HOLD при failed/unknown outcome; сохранить stage-2 safety invariants.
4. **Не** менять API/contracts без синхронного обновления `core-rpc`, клиентов и UI tests.
5. Сначала реализовать чистый `KmiResolution` и тесты; затем провести ограниченный preflight-эксперимент на C и ядрах, только потом wiring и production rollout.
6. Проверить реальные кодовые пути `Prepared::scan/execute`, `Journal::check_native_attempt`, `load_kmod`, `payloads::selection::available`, UI `buildNativeInstallRecommendation/classifyKmodProblem`, а не строить вторую параллельную систему.
7. В каждом PR указывать, какие проверки реально выполнены: unit, CI build, QEMU, Android device. **Не** объявлять новую автоматику production-safe на основании только host-тестов.
8. После успешного merge обновить существующий документ `docs/notes/unified-core-implementation.md` и пользовательскую документацию о едином Core; этот RFC остаётся record of decisions.

---

## 15. Источники и привязки к коду

### Репозиторий (snapshot `dev-2.x`)

- [Текущий unified Core implementation plan](https://github.com/okhsunrog/vpnhide/blob/e50134df6cf513b54eea12add73084af6a730f42/docs/notes/unified-core-implementation.md)
- [Android recommendation / kmod diagnosis](https://github.com/okhsunrog/vpnhide/blob/e50134df6cf513b54eea12add73084af6a730f42/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/DashboardData.kt)
- [Payload manifest & verification](https://github.com/okhsunrog/vpnhide/blob/e50134df6cf513b54eea12add73084af6a730f42/crates/core-daemon/src/payloads/mod.rs)
- [Pure payload selector](https://github.com/okhsunrog/vpnhide/blob/e50134df6cf513b54eea12add73084af6a730f42/crates/core-daemon/src/payloads/selection.rs)
- [Unified payload assembly](https://github.com/okhsunrog/vpnhide/blob/e50134df6cf513b54eea12add73084af6a730f42/scripts/payload_bundle.py)
- [Native boot orchestration](https://github.com/okhsunrog/vpnhide/blob/e50134df6cf513b54eea12add73084af6a730f42/crates/core-daemon/src/native_boot.rs)
- [Prepared effect snapshot](https://github.com/okhsunrog/vpnhide/blob/e50134df6cf513b54eea12add73084af6a730f42/crates/core-daemon/src/native_boot/prepared.rs)
- [Native journal & budget](https://github.com/okhsunrog/vpnhide/blob/e50134df6cf513b54eea12add73084af6a730f42/crates/core-daemon/src/journal.rs)
- [Loader wrapper](https://github.com/okhsunrog/vpnhide/blob/e50134df6cf513b54eea12add73084af6a730f42/crates/activator/src/load_kmod.rs)
- [Kernel module init](https://github.com/okhsunrog/vpnhide/blob/e50134df6cf513b54eea12add73084af6a730f42/kmod/vpnhide_kmod.c)
- [Kernel module installation guide (старый пользовательский сценарий)](https://github.com/okhsunrog/vpnhide/blob/e50134df6cf513b54eea12add73084af6a730f42/docs/help/ru/kmod-install.md)

### Семантика Linux / Android

- [init_module(2), finit_module(2): включая возврат errno из module init](https://man7.org/linux/man-pages/man2/init_module.2.html)
- [Linux module init cleanup: `do_init_module` и negative init result](https://sources.debian.org/src/linux/6.1.4-1/kernel/module/main.c)
- [Kernel documentation: building external modules, CONFIG_MODVERSIONS, Module.symvers](https://www.kernel.org/doc/html/latest/kbuild/modules.html)
- [Android: Maintain a stable kernel module interface](https://source.android.com/docs/core/architecture/kernel/stable-kmi)
- [Android: Common kernels и naming GKI branches](https://source.android.com/docs/core/architecture/kernel/android-common)
- [Пример `same_magic()` с частичным игнорированием release при наличии CRC](https://android.googlesource.com/kernel/common/+/refs/tags/android15-6.6-2024-08_r11/kernel/module/version.c)

---

## 16. Итоговое решение

**Принято в качестве предлагаемой архитектуры:**

- **Один Core ZIP**, все поддерживаемые `.ko` внутри, пользователь ничего не выбирает.
- **Пассивное определение кандидатов** по проверенным источникам, Linux LTS и manifest.
- **Loader preflight того же финального `.ko`** с ранним отрицательным `module_init` — перспективный, но пока **экспериментальный** механизм; не считать полным доказательством ABI и не включать без kernel/device validation.
- **Один полноценный real load** после выбора и прочных checks; никаких слепых повторов после ошибки/краша.
- **Консервативное разрешение неоднозначности**: если данных недостаточно — другой доказанно безопасный backend или понятный fail-closed outcome, а не угадывание.
- **Existing durable lifecycle first:** single writer, prepared bytes, journal/holds, owner reconciliation и корректная диагностика обязательны.
- **UI без технической нагрузки:** «Защита активна» только при подтверждённом статусе, автоматическое исправление — только при существовании безопасного исполнимого плана.

