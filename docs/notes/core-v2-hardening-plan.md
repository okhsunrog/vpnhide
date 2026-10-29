# План улучшения архитектуры VPNHide v2

Дата: 2026-10-29. База ревью: `dev-2.x`, коммит `bab2ee37`.

Статус: живой план, а не описание полностью реализованной архитектуры.
Исходные проблемы ниже относятся к базе `bab2ee37`; первый проход исправлений —
[`9613e21a`](https://github.com/okhsunrog/vpnhide/commit/9613e21a9fcdb42c19fc250f83aab9d7761099a2).
Текущий контракт описан в [core-architecture.md](../core-architecture.md).
Отмеченный checkbox означает реализованный код/host regression, а не device acceptance.
D2 проверен 128 сочетаниями ресурса/holds/retention с повторной загрузкой journal.
Повторный общий Rust-прогон: 284 passed; Clippy/fmt прошли. Первый общий прогон
имел KPM EAGAIN (подробности B3); позже конкуренция была воспроизведена
отдельно и исправлена в `30380d65`.
Повторены 868 JVM-тестов, ktlint/detekt/CPD/Android lint; Gradle test+assembleDebug
завершился успешно. Core ZIP собран для arm64/armv7. Это по-прежнему host-only
проверки и APK с временной тестовой подписью, а не device acceptance.

## Сводка выполнения

| Пункт | Статус и evidence | Остаток |
|---|---|---|
| A1 | Реализовано: `export.rs`, descriptor-relative публикация, приватный staging; host-тесты symlink/подмены родителя/timeout | DAC/SELinux и crash на Android не проверены |
| A2 | Частично: `core-rpc/src/recovery.rs`, typed FFI/UI, матрица 32 holds; startup fault виден в UI | Структурированный fault в bundle; дальнейшее упрощение coordinator |
| A3 | Реализовано: `config_file/codec.rs`, строгие типы/версия/blank-file, сохранение ошибочного источника | Device acceptance импорта |
| A4 | Реализовано (`c2ddec74`): storage fault отдельно от terminal receipt; fault matrix до/после rename и restart | RPC v12 требует совместимых APK/core; device fault acceptance |
| A5 | Реализовано (`15d43f07`): pending/active внутри lifecycle, подтверждённый logging, Instant deadlines, orphan cleanup | Device capture/reboot acceptance |
| B1 | Исправлены доказанные переполнения: disk+RPC budget, полный envelope, escaped diagnostics | Компактный отдельный config fetch — предложение, не реализовано |
| B2 | `574eae03`: реализовано разделение durable admission/execution; CommandEngine владеет store, mailbox и writer; UI ждёт receipt | Queue/admission latency и отдельная подготовка остаются связанными с B3 |
| B3–B4 | Не реализованы целиком | Handlers/deadlines, профили запуска; device validation KPM gate |
| C1–C3 | Не реализованы | Provenance, export lifecycle, measurement identity |
| D2 | [`fbc60b6c`](https://github.com/okhsunrog/vpnhide/commit/fbc60b6c): реализована проверка зависимостей boot внутри writer; матрица holds/retention/restart в `command_tests.rs` | Реальный reboot и независимый Ports repair на устройстве |
| D1, D3 | Не реализованы | Runtime reconciliation, служебная durability/логи |
| Миграции 0.x/1.x | Изолированы в `migration/`, реальные старые форматы и fixtures; [контракт](../migration.md) | Прямой upgrade на устройстве |
| Остальной этап 5 | Предложения, не сделаны | Порядок и оценка ниже |

Проверки первого прохода: 283 Rust-теста и 868 JVM-тестов, Clippy/fmt,
ktlint/detekt/CPD/lint, APK и core ZIP. Это host-проверки; APK подписан временным
тестовым ключом. Device acceptance отсутствует. GitHub CI для прямого push в
`dev-2.x` автоматически не запускается; ручной dispatch не создан из-за Forbidden.
Эти результаты не являются зелёным GitHub CI и не подтверждают готовность релиза.

Цель: устранить нарушения границ privileged API и восстановления, завершить
перенос runtime ownership в core, затем убрать дублирующие модели и легаси.
P1 означает первый этап исправлений; P2 — следующий приоритет. Архитектурные
предложения отдельно отмечены и требуют проверки совместимости при реализации.

## Инварианты, которые сохраняем

- Единственный привилегированный writer и durable intent перед внешним эффектом.
- Выполнение принятой операции независимо от времени жизни UI и соединения.
- Настоящий typed UniFFI, отдельный core-client, отсутствие daemon/activator в APK.
- Fencing по epoch/generation и отдельные config/publication revisions.
- Разделение desired config, applied policy и measured behavior.
- Секреты не попадают в journal, диагностические payload, argv или логи.
- Неопределённые эффекты не повторяются автоматически без достаточных оснований.

## Этап 1. Границы privileged API и безопасное восстановление

### A1. P1: закрыть перенаправление root-экспорта

Исходная проблема (до первого исправления): подтверждена исходниками и локальным воспроизведением копирования через symlink.
В [diagnostics.rs](../../crates/core-daemon/src/diagnostics.rs) каталог передавался
app UID до запуска root child. Далее
[lsposed.rs](../../crates/core-daemon/src/device_probes/lsposed.rs) и
[kernel.rs](../../crates/core-daemon/src/device_probes/kernel.rs) открывали
назначения обычными файловыми операциями, следующими по symlink.
Уже авторизованный UID приложения мог перенаправить root-запись за пределы
назначенного экспорта. Это не утверждение о доступе произвольного другого APK.

- [x] Собирать артефакты в root-owned staging вне app-writable дерева.
- [x] Публиковать только завершённые файлы; использовать проверенные дескрипторы
  каталогов и файлов, исключить подмену родителя и следование по symlink.
- [x] Изменять ownership/mode/label через дескриптор, где это возможно.
- [ ] Проверить очистку staging при timeout, cancellation, crash и отказе записи.

Готово, когда тесты подмены файла и каталога не меняют внешний контрольный файл,
а обычные экспорты доступны приложению с ожидаемыми DAC/SELinux атрибутами.

### A2. P1: сделать recovery доступным из приложения

Исходная проблема (до первого исправления): подтверждена трассировкой исходников.
[ConfigCoordinator.kt](../../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/ConfigCoordinator.kt)
при mutationHeld запрещал почти все команды, а canRepair был разрешён лишь для
Native/Ports. Поэтому восстановительные команды для Secret/Cleanup/Persist,
которые core мог безопасно принять, оставались недоступными через UI.

- [x] Определить в core типизированные доступные recovery actions и причины отказа.
- [x] Использовать общий модуль recovery-policy для admission, boot и UI:
  boot автоматически допускает только независимые ресурсы, explicit repair
  имеет отдельные разрешения и повторно проверяется core.
- [ ] Уменьшить app coordinator до intent/draft/conflict adapter; убрать повторное
  вычисление права на recovery из общего Paused.
- [ ] Сохранить структурированную причину startup fault в CoreStateView и bundle.
- [x] Проверить доступность Repair, Set/ClearSuperkey, Cleanup и FullReset для
  каждого допустимого сочетания held resources; запретить небезопасные сочетания.

Готово, когда для каждого hold UI показывает причину и все допустимые действия,
а Recheck не выдаётся за способ снять неизменяемый terminal Unknown.

### A3. P2: строго проверять canonical при загрузке

Исходное воспроизведение (исправлено): [config_file.rs](../../crates/core-daemon/src/config_file.rs)
принимал `null`, `[]`, `{"apps":[]}` как Present с пустым apps. Значение
`{"version":4294967297}` сужалось до версии 1. Boot apply мог применить пустую
политику вместо перехода в явно неисправное состояние.

- [x] Проверять тип корневого значения и присутствующих полей до нормализации.
- [x] Использовать проверяемое преобразование версии без переполнения.
- [x] Отделить поддержанные старые формы от семантически неправильных значений.
- [x] При повреждении сохранять файл и публиковать startup fault без boot delivery.

Готово, когда неправильные формы и версии отказывают явно, а все поддержанные
старые файлы читаются без потери смысла.

### A4. P2: различить durable receipt и недолговечную неопределённость

Исходное воспроизведение (исправлено) при отказе journal temp write после Persist: get_operation
возвращает settled Unknown, а после рестарта тот же ID становится Confirmed.
В [server.rs](../../crates/core-daemon/src/server.rs) старый fallback менял результат
только в памяти, тогда как на диске остаётся предшествующая запись.

- [x] Определить отдельную семантику storage fault и недоступного terminal receipt.
- [x] Не публиковать несохранённый результат как durable terminal Unknown.
- [x] Сохранить первоначальный pending intent для последующей проверки evidence.
- [x] Добавить fault matrix: отказ до/после rename, after-Persist, settlement,
  повторное чтение и рестарт после уже отправленного клиенту ответа.

Готово, когда durable terminal receipt неизменяем, а временная неопределённость
явно отличается от него во всём API и UI.

### A5. P2: восстановить инвариант debug lease

Исходное воспроизведение (исправлено): lease сохранялась до включения debug. После неудачного включения
повторный acquire того же ID лишь продлевает срок и возвращает Ok при debug=false.

- [x] Разделить Pending/Active либо проверять обеспечение debug при renewal.
- [x] Определить поведение повторного acquire после FailedKnown и Unknown.
- [x] Сохранить baseline и обязательство восстановления при всех отказах.
- [x] Использовать monotonic deadline внутри boot; wall time оставить для
  отображения. Для рестарта определить политику с учётом boot identity.

Готово, когда успешная активная lease гарантирует обещанное состояние logging,
а сдвиг системных часов не продлевает orphan lease и не снимает живую раньше срока.

## Этап 2. Контракт RPC, admission и ресурсные ограничения

### B1. P1: согласовать размеры config и исходящих кадров

Исходное воспроизведение (исправлено) на штатном daemon/dump: конфиг с 6300 Java-only приложениями
занимает 510521 байт на диске и проходит validate_candidate, но его нормализованное
RPC-представление занимает 1077521 байт при лимите кадра 1048576 байт.
Handshake проходит, чтение состояния закрывает канал. Состояние достижимо
небольшими Batch или загрузкой файла, даже если большой Replace отсечётся на входе.

- [x] Ограничивать полный encoded response с учётом metadata и receipts.
- [x] Проверять публикуемость нового состояния до его сохранения.
- [ ] Спроектировать компактную publication; большой config при необходимости
  получать отдельно с согласованной revision и ограниченной передачей.
- [x] Для diagnostics считать encoded bytes, включая JSON escaping; переносить
  избыток в безопасный файловый экспорт.

Синтетическая проверка diagnostics: 640 KiB строк с кавычками дают JSON report
1311575 байт. Это доказательство ошибки бюджета, не запись реального probe.

Готово, когда любой принятый config остаётся читаемым, а допустимые ответы никогда
не рвут общий канал из-за превышения frame limit. Увеличения константы недостаточно.

### B2. P2: отделить durable admission от execution

Исходная проблема: submit_command ждал execute_command целиком при клиентском
дедлайне 15 секунд. Теперь typed CommandEngine отделяет admission waiter от
владельца исполнения. После проверки конфликтов/capacity и sync первого phase
intent возвращается Accepted до внешнего эффекта. UI остаётся pending и наблюдает
receipt без повторной отправки. Внутренние boot/lease команды сохраняют ожидание
completion. Очередь ограничена 16 сообщениями, при заполнении возвращается Busy.

Подготовка/capacity checks и ожидание очереди ещё предшествуют admission: их
latency и ID allocation могут ждать занятого writer. Поэтому ранний Accepted не
закрывает B3 и не обещает отсутствие всех transport timeouts. Новый тест проверяет
ответ до двухсекундного эффекта и завершение после disconnect; crash fixtures
принимают как доставленный Accepted, так и потерю ответа, сверяя один и тот же ID.

- [x] Возвращать Accepted после durable admission, до завершения внешних эффектов.
- [ ] Явно представить подготовку, прогресс и terminal result операции.
- [x] Сохранить field conflict и capacity semantics; определить, какие отказы
  относятся к admission, а какие завершают уже принятую операцию.
- [x] Проверить очередь, disconnect и рестарт между admission и первым эффектом.

Готово, когда долгая штатная операция наблюдается как прогресс, а потеря ответа
восстанавливается по тому же ID без повторения effects.

### B3. P2: ограничить обработчики и согласовать сквозные дедлайны

- [ ] Ограничить число RPC handlers внутри соединения и глобально: текущий
  for_each(tokio::spawn) не покрывается лимитом восьми соединений или writer queue.
- [ ] Сохранить доступность state/receipt reads при перегрузке тяжёлой работой.
- [ ] Согласовать root-cache deadline 15 секунд с разрешённым временем core probe
  и parent-side KPM observation; проверить late completion и quarantine.
- [ ] Выделить быстрый status/inventory из тяжёлого forensic read path.

Готово, когда нагрузочный тест показывает ограниченное число задач и объём памяти,
а нормальная работа в разрешённом бюджете не объявляется timeout другим слоем.

KPM observation/delivery contention был подтверждён детерминированным тестом
в `e316470b`: read_state/status мог держать flock, из-за чего delivery получала
BeforeDispatch/EAGAIN. Теперь локальный per-path gate отделяет конкуренцию внутри
core от внешнего владельца lock. Observation остаётся неблокирующей; load/delivery
ждёт локального владельца не более 10 секунд **до первого dispatch**. Внешний flock
остаётся nonblocking, а неизвестный результат отправленного эффекта не повторяется.
Регрессии: `delivery_waits_for_local_observation_and_dispatches_once` и
`external_control_lock_still_refuses_delivery_before_dispatch`. Это исправление
локальной конкуренции, не завершение всех B3 deadlines или device acceptance.

### B4. P2: сделать профили запуска и подготовку socket явными

Штатный service задаёт AppOwner. Ручной запуск допускает опасное сочетание:
production config/journal paths, Open allowlist и socket mode 0666. Ошибочная
UID-строка также превращается в Open. Доступ на Android дополнительно зависит
от SELinux; это footgun ручного запуска, а не доказанная экспозиция штатного service.

- [ ] Ввести явные Production/Sandbox настройки, прочитанные один раз при запуске.
- [ ] Для Sandbox требовать изолированные пути и принимать свой UID по умолчанию.
- [ ] Для Production требовать AppOwner и завершённую подготовку DAC/SELinux.
- [ ] Ошибку chown/chmod/relabel считать ошибкой bind с повтором и диагностикой,
  а не успешным listening; host-only отсутствие SELinux обрабатывать отдельно.

## Этап 3. Согласованность измерений и экспортов

### C1. P2: сохранять provenance измерения до самого bundle

- [ ] В LogcatRecorder добавлять секцию через copy существующего RootSnapshot,
  сохраняя core publication, native boot и observation identity.
- [ ] Передавать в export завершённое DiagnosticMeasurement с coverage/context
  либо готовый отчёт, не сокращая его до gate/results/runId.
- [ ] Не атрибутировать старые results заново по текущему backend/LSPosed state.
- [ ] Хранить текущее наблюдение отдельно от условий выполненного измерения.

Исходники:
[LogcatRecorder.kt](../../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/debug/LogcatRecorder.kt),
[DebugExportData.kt](../../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/debug/DebugExportData.kt),
[VpnHideState.kt](../../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/debug/VpnHideState.kt).

Готово, когда logcat bundle сохраняет boot/load ошибки, а смена backend между suite
и export не меняет атрибуцию завершённого измерения.

### C2. P2: дать экспорту process-owned handle и результат

- [ ] Ввести один наблюдаемый ExportState со стабильным capture ID и результатом.
- [ ] Позволить экрану принять существующую работу после Activity recreation.
- [ ] Ограничить пересекающиеся forensic windows UI/bridge: dmesg и счётчики общие.
- [ ] Создавать уникальные ZIP без truncate существующего файла; точности секунды
  в имени недостаточно.
- [ ] Проверить отмену waiter, ошибки упаковки, повторное сохранение и очистку.

Использовать локальный подход LogcatRecorder; универсальный workflow framework
для этой задачи не требуется.

### C3. P2: отвязать measurement identity от истории команд

[DesiredDeliveryData.kt](../../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/diagnostics/DesiredDeliveryData.kt)
включает глобальные operation ID/revision в identity. Нерелевантные изменения
debug или чужих приложений могут вызвать новый suite; debug lease самого capture
способна создать лишнюю диагностику в его counter window.

- [ ] Строить identity по effective policy собственного UID и backend generation.
- [ ] Сохранить receipt ID как evidence исполнения, отдельно от measurement key.
- [ ] Согласовать правила с DiagnosticImpact и admission измерений.

Готово, когда нерелевантные изменения не запускают suite, а изменение собственной
политики, backend или routing корректно инвалидирует measurement.

## Этап 4. Завершение runtime ownership в core

### D1. P2: пересчитывать UID-проекции и обнаруживать потерю Ports

Сейчас UID всех targets разрешаются при apply. Observer следит за сетью и UID
самого VPNHide, но не за UID всех целевых пакетов. После удаления/переустановки
или появления profile copy проекция может устареть. Ports получает boot pass и
повтор через 30 секунд, но не проверяет позднюю потерю правил.

- [ ] Наблюдать изменение package inventory и инвалидировать affected projections.
- [ ] Добавить bounded сверку собственных Ports chains/jumps и нужной policy.
- [ ] Выполнять восстановление новой операцией с учётом held resources.
- [ ] Перенести auto-hide VPN-app reconciliation из зависимости от запуска UI.
- [ ] Не трактовать исторический successful receipt как текущую runtime liveness.

Готово, когда при закрытом UI установка/удаление/profile change и потеря firewall
правил приводят к корректной сверке либо явному состоянию невозможности защиты.

### D2. P2: восстанавливать независимые ресурсы при boot

Исходная проблема: boot::reapply_with останавливал весь apply при любом hold.
Теперь boot ставит новую RepairActivation с ограниченным планом; внутри writer
проверяется текущий journal (включая expired holds). Persist блокирует любой apply;
Native также зависит от Secret и Native, Ports — от Ports. Cleanup не блокирует
независимую доставку. Автоматический boot не делает Persist readback и не снимает
неопределённость зависимого ресурса: для этого остаётся explicit recovery.
Первый и повторный Ports pass проходят одну и ту же проверку.

- [x] Использовать таблицу зависимостей из A2 для выбора разрешённых boot repairs.
- [x] Сохранить исходные Unknown receipts и запретить повтор старых secret/cleanup
  effects под видом восстановления независимой политики.

### D3. P2: усилить служебную durability и ограничить логи

- [ ] Ограничить и ротировать core.log с корректным переоткрытием stdout/stderr.
- [ ] Сохранять fixed-per-boot selection общим durable-file primitive с fsync;
  при отказе не заявлять гарантию сохранённого решения.
- [ ] Проверить crash/restart supervisor и восстановление при storage failure.

## Этап 5. Упрощение моделей и удаление легаси

Архитектурные предложения. Выполнять отдельными изменениями после исправления
контрактов выше; не совмещать механические переносы с изменением recovery semantics.

- [ ] **Компактировать journal.** Полные candidate/native intent сохранять там,
  где они нужны pending/held recovery. Terminal history сделать компактной;
  продление lease не должно переписывать 64 полных candidate configs. Для дедупа
  несекретных команд рассмотреть canonical fingerprint. Не сохранять verifier
  секрета. Отделить hot lease metadata от тяжёлой истории.
- [ ] **Довести writer до владения состоянием.** Частично выполнено: CommandEngine
  владеет ConfigStore, typed command mailbox и жизненным циклом writer; immutable
  publication отделена от execution. Короткие mutex sections и working clones
  вокруг IO пока сохранены. Не отмечать весь пункт завершённым: удаление остальных
  clone-путей и дальнейшее отделение executor от CoreState — следующий рефакторинг.
- [ ] **Типизировать обычные root reads.** RuntimeInventory, ModuleStatus и
  PackageInventory должны заменить section-name mini-protocol в Dashboard/Hiding.
  Raw sections сохранить как forensic content.
- [ ] **Убрать JSON-мост core → activator.** Проекция должна принимать общий
  нормализованный тип непосредственно, без serialize/parse другой модели.
- [ ] **Сократить зеркальные API-схемы.** Выделить стабильные DTO с serde и
  optional UniFFI derives, проверить BTreeMap/BTreeSet normalization. Удалить
  ненужные обратные From; сохранять domain/storage conversion по смыслу.
- [ ] **Убрать app JSON round-trip из обычного typed edit path.** Оставить чистую
  доменную валидацию и авторитетный core validator; codec equivalence закрепить
  отдельными тестами.
- [x] **Изолировать перенос 0.x и 1.x.** Поддержка обоих поколений сохраняется:
  пользователь подтвердил необходимость прямого перехода без промежуточной
  установки 1.x. App-адаптеры находятся в `migration/`, выдают текущую typed
  модель и используют общий core validator/Persist. Формат schema 1 остаётся
  текущим контрактом LSPosed, а не удаляемым runtime legacy. Исходники неполного
  импорта сохраняются. См. [границу миграций](../migration.md).
- [ ] **Убрать ownership markers без действующей fencing роли.** Сначала проверить
  всех внешних потребителей и upgrade path; diagnostic ownership получать из core.
- [ ] **Удалить устаревшие исторические design notes.** Перенести полезные актуальные
  сценарии в текущие контракты, убрать ссылки из AGENTS/index. Кандидаты:
  app-state-design, app-state-transitions-history, diagnostics-state-analysis,
  исторический ui-state-presentation-review. Историю сохраняет Git.

Canonical JSON v1 нельзя удалить без согласованной миграции LSPosed и остальных
читателей. Проверки старых Android API, включая NetworkInfo, остаются частью
реального detection coverage. Замена внутреннего JSON codec tarpc на binary
не является приоритетом без измеренного выигрыша.

## Порядок реализации и проверка результата

1. Завершённый проход: A1/A3, доказанные случаи B1, UI recovery из A2 и изоляция
   миграций. Следующий небольшой шаг D2 устраняет независимый boot blocker без
   изменения RPC и формата journal.
2. A4/A5 выполнены отдельными коммитами с регрессиями. Они меняют смысл результата
   и lifetime; перед beta остаётся device acceptance, а не повторный перенос кода.
3. B2: первый CommandEngine и ранний Accepted реализованы с сохранением
   conflict/dedup/retention, queue/shutdown tests и новым UI ожиданием. Остаток —
   preparation/queue latency и дальнейшее сокращение clone/store путей.
4. B3/B4: ограничить handlers, проверить reads под нагрузкой, согласовать deadlines
   и профили запуска. D1 — после этого bounded reconcile через тот же engine;
   автоматическое добавление RU-приложений не входит в beta scope.
5. C1–C3 и D3 остаются необходимыми проверками перед соответствующими beta
   сценариями; не считать диагностику достоверной только потому, что ZIP создан.
6. Затем остальные упрощения этапа 5 отдельными проверяемыми изменениями.

### Оценка предложенного рефакторинга

- **CommandEngine — оправдан.** Уменьшает shared mutable state и clone/store пути,
  даёт ясную границу B2. Не нужен универсальный workflow framework; thin RPC
  adapter и конкретные typed messages достаточны. Перенос кода не должен сам
  менять receipt semantics или разрешать повтор effects.
- **Journal — оправдано компактировать после A4.** Нужны crash/fault tests,
  доказанная сохранность pending evidence и совместимость чтения старого journal.
  Сначала терминальная семантика, потом размер/частота записи, не наоборот.
- **DTO — оправдано уменьшить бессмысленные зеркала.** Domain/storage/API границы
  различаются по смыслу: не объединять их ради одной структуры. Проверить
  normalization BTreeMap/BTreeSet и генерируемый UniFFI до удаления converters.
- **Typed inventory и удаление JSON round-trip — полезны отдельными этапами.**
  Runtime reads могут стать typed, raw sections остаются forensic content;
  canonical schema 1 остаётся текущим LSPosed контрактом. Миграционные adapters
  0.x/1.x сохраняются, старые runtime activators не возвращаются.
- **Transport rewrite — отложен.** tarpc/JSON сейчас не доказан причиной
  correctness-дефектов. Сначала budgets, admission и handler limits; переход
  на binary допускается после измерений, а не как обязательный beta gate.

Для каждого изменения обновлять текущий контракт и связанные лимиты. При изменении
app/core wire согласованно обновлять protocol version, bindings и обе поставки;
backend control-v2/telemetry-v1 не менять без отдельной необходимости. Для изменения
bundle shape/semantics следовать правилам schema/golden в lsposed/AGENTS.md.

Проверки по направлениям:

- Rust: targeted regression tests, затем соответствующие CI test/clippy/fmt gates.
- Android: JVM regressions на recovery holds, recreation/concurrent export,
  snapshot provenance, смену backend при export и нерелевантные config changes;
  ktlint, detekt, CPD, lint и сборка APK.
- Durability: fault matrix до/после rename, fsync, settlement и restart; доказать
  отсутствие повторного dispatch и сохранение смысла уже отправленных receipts.
- Resource bounds: worst-case encoded frames, параллельные long polls/probes,
  bounded allocation, очистка файлов и восстановление после timeout.
- Device acceptance: реальные DAC/SELinux, root-manager boot/restart, медленные
  PM/KPM, package/profile lifecycle и потеря Ports при закрытом UI. Конкретный
  сценарий сброса firewall выполнять только на подготовленном тестовом устройстве.

При исходном ревью прошли 262 теста core/client/FFI. Внешними примерами проверены
oversized config с daemon/dump, symlink destination, повтор lease после ошибки,
изменение receipt после storage failure/restart и неправильные JSON-формы.
Android/SELinux/device acceptance этим не установлены. Это исходная проверка
базы ревью, а не подтверждение выполнения будущих пунктов плана.

## Выполненный ограниченный проход перед бетой

Исправлены исходные четыре сценария ревью: строгая загрузка canonical (A3),
размеры config/publication и JSON-escaping диагностики (B1), root-owned staging
и descriptor-based export (A1), доступ к разрешённым recovery-командам через UI
(A2). Это не означает выполнения всех подпунктов соответствующих разделов:
D2 реализован следующим отдельным шагом; структурированный startup-fault
в bundle ещё требует изменения. Перенос 0.x/1.x изолирован без возврата старых активаторов.

Минимальный beta gate: зелёные host/JVM/lint проверки и на тестовом Android
проверенные чистая установка, прямой перенос 0.x/1.x, reboot/restart, сохранение
и repair при потере соединения, файловый export с реальными DAC/SELinux и работа
выбранного backend/Ports с закрытым APK. Без устройства эти сценарии не считаются
проверенными. B3/D1 и остаток B2 остаются открытыми; A4/A5/D2 требуют device acceptance; большой CommandEngine/DTO
rewrite не является обязательным условием этой ограниченной серии исправлений.


## Открытые gates первой надёжной beta

- **Correctness:** A4/A5 закрыты host-регрессиями; проверить отказы хранения и
  lifecycle на устройстве. Проверить на устройстве согласованный KPM observation/delivery gate
  и его timeout; host-тесты подтверждают отсутствие повторного dispatch.
- **Основные сценарии:** B2/B3 должны выдерживать реальные длительные операции,
  disconnect и нагрузку без ложного повторения effects; D1 — package/profile
  change и потерю Ports при закрытом APK. Пока этого evidence нет, соответствующая
  надёжность не подтверждена.
- **Device acceptance:** чистая установка, прямой импорт 0.x/1.x, reboot/restart,
  recovery при held resources, DAC/SELinux экспорта и выбранный backend. Host
  matrix D2 подтверждает admission/retention, но не реальную доставку firewall.
- **Поставка:** совместимые APK/core, обычная подпись для upgrade, проверенные
  артефакты точного кандидата. Тестовый APK и отсутствие GitHub CI не выдаются за
  пройденный release gate. RU auto-add и полный transport rewrite сюда не входят.


## Завершённая итерация A4 → A5 → CommandEngine

- [`c2ddec74`](https://github.com/okhsunrog/vpnhide/commit/c2ddec74): runtime storage
  fault отдельно от durable terminal receipt; RPC/FFI v12 и UI blocked feedback.
- [`15d43f07`](https://github.com/okhsunrog/vpnhide/commit/15d43f07): pending lease
  не выдаётся за успешную active; монотонный lifetime и восстановление baseline.
- [`e316470b`](https://github.com/okhsunrog/vpnhide/commit/e316470b): воспроизведение
  локальной KPM contention; [`30380d65`](https://github.com/okhsunrog/vpnhide/commit/30380d65)
  исправляет её через ограниченное ожидание до dispatch, без повторения эффекта.
- [`574eae03`](https://github.com/okhsunrog/vpnhide/commit/574eae03): CommandEngine,
  durable Accepted до effects, ожидание terminal receipt в приложении.

Итоговые проверки: **349 Rust-тестов** (activator/core-rpc/core-daemon/core-client/
core-ffi), Clippy all-targets с `-D warnings`, cargo fmt; **870 JVM-тестов**,
ktlint, detekt, CPD, Android lint и assembleDebug. Core ZIP собран для arm64/armv7;
module scaffold, no-root-exec, changelog preview и ссылки изменённых документов
проверены. Ранние неуспешные прогоны выявили старые тестовые предположения
«Accepted = completed» и KPM contention; финальный общий прогон проходит целиком.

APK остаётся подписанным временным тестовым ключом. Device acceptance и GitHub CI
не выполнены; запрещённый ручной dispatch не повторялся. Отсутствие device/CI
нельзя заменять количеством host-тестов. Следующий scope: B3 (queue/admission
latency, handlers, deadlines), B4 и D1; дальнейшее сокращение clone/store путей
выполнять отдельно, не возвращая app-owned effects или старые активаторы.
