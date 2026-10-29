# План улучшения архитектуры VPNHide v2

Дата: 2026-10-29. База ревью: `dev-2.x`, коммит `bab2ee37`.

Статус: план реализации по результатам исходникового ревью и отдельных
воспроизведений. Пункты ниже ещё не реализованы этим документом. Текущий контракт
описан в [core-architecture.md](../core-architecture.md); предложения этого плана
не следует принимать за уже существующее поведение.

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

Подтверждено исходниками и локальным воспроизведением копирования через symlink.
В [diagnostics.rs](../../crates/core-daemon/src/diagnostics.rs) каталог передаётся
app UID до запуска root child. Далее
[lsposed.rs](../../crates/core-daemon/src/device_probes/lsposed.rs) и
[kernel.rs](../../crates/core-daemon/src/device_probes/kernel.rs) открывают
назначения обычными файловыми операциями, следующими по symlink.
Уже авторизованный UID приложения может перенаправить root-запись за пределы
назначенного экспорта. Это не утверждение о доступе произвольного другого APK.

- [ ] Собирать артефакты в root-owned staging вне app-writable дерева.
- [ ] Публиковать только завершённые файлы; использовать проверенные дескрипторы
  каталогов и файлов, исключить подмену родителя и следование по symlink.
- [ ] Изменять ownership/mode/label через дескриптор, где это возможно.
- [ ] Проверить очистку staging при timeout, cancellation, crash и отказе записи.

Готово, когда тесты подмены файла и каталога не меняют внешний контрольный файл,
а обычные экспорты доступны приложению с ожидаемыми DAC/SELinux атрибутами.

### A2. P1: сделать recovery доступным из приложения

Подтверждено трассировкой исходников.
[ConfigCoordinator.kt](../../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/ConfigCoordinator.kt)
при mutationHeld запрещает почти все команды, а canRepair разрешён лишь для
Native/Ports. Поэтому восстановительные команды для Secret/Cleanup/Persist,
которые core может безопасно принять, остаются недоступными через UI.

- [ ] Определить в core типизированные доступные recovery actions и причины отказа.
- [ ] Использовать одну политику зависимостей ресурсов для admission, boot и UI.
- [ ] Уменьшить app coordinator до intent/draft/conflict adapter; убрать повторное
  вычисление права на recovery из общего Paused.
- [ ] Сохранить структурированную причину startup fault в CoreStateView и bundle.
- [ ] Проверить доступность Repair, Set/ClearSuperkey, Cleanup и FullReset для
  каждого допустимого сочетания held resources; запретить небезопасные сочетания.

Готово, когда для каждого hold UI показывает причину и все допустимые действия,
а Recheck не выдаётся за способ снять неизменяемый terminal Unknown.

### A3. P2: строго проверять canonical при загрузке

Воспроизведено: [config_file.rs](../../crates/core-daemon/src/config_file.rs)
принимает `null`, `[]`, `{"apps":[]}` как Present с пустым apps. Значение
`{"version":4294967297}` сужается до версии 1. Boot apply может применить пустую
политику вместо перехода в явно неисправное состояние.

- [ ] Проверять тип корневого значения и присутствующих полей до нормализации.
- [ ] Использовать проверяемое преобразование версии без переполнения.
- [ ] Отделить поддержанные старые формы от семантически неправильных значений.
- [ ] При повреждении сохранять файл и публиковать startup fault без boot delivery.

Готово, когда неправильные формы и версии отказывают явно, а все поддержанные
старые файлы читаются без потери смысла.

### A4. P2: различить durable receipt и недолговечную неопределённость

Воспроизведено при отказе journal temp write после Persist: get_operation
возвращает settled Unknown, а после рестарта тот же ID становится Confirmed.
В [server.rs](../../crates/core-daemon/src/server.rs) fallback меняет результат
только в памяти, тогда как на диске остаётся предшествующая запись.

- [ ] Определить отдельную семантику storage fault и недоступного terminal receipt.
- [ ] Не публиковать несохранённый результат как durable terminal Unknown.
- [ ] Сохранить первоначальный pending intent для последующей проверки evidence.
- [ ] Добавить fault matrix: отказ до/после rename, after-Persist, settlement,
  повторное чтение и рестарт после уже отправленного клиенту ответа.

Готово, когда durable terminal receipt неизменяем, а временная неопределённость
явно отличается от него во всём API и UI.

### A5. P2: восстановить инвариант debug lease

Воспроизведено: lease сохраняется до включения debug. После неудачного включения
повторный acquire того же ID лишь продлевает срок и возвращает Ok при debug=false.

- [ ] Разделить Pending/Active либо проверять обеспечение debug при renewal.
- [ ] Определить поведение повторного acquire после FailedKnown и Unknown.
- [ ] Сохранить baseline и обязательство восстановления при всех отказах.
- [ ] Использовать monotonic deadline внутри boot; wall time оставить для
  отображения. Для рестарта определить политику с учётом boot identity.

Готово, когда успешная активная lease гарантирует обещанное состояние logging,
а сдвиг системных часов не продлевает orphan lease и не снимает живую раньше срока.

## Этап 2. Контракт RPC, admission и ресурсные ограничения

### B1. P1: согласовать размеры config и исходящих кадров

Воспроизведено на штатном daemon/dump: конфиг с 6300 Java-only приложениями
занимает 510521 байт на диске и проходит validate_candidate, но его нормализованное
RPC-представление занимает 1077521 байт при лимите кадра 1048576 байт.
Handshake проходит, чтение состояния закрывает канал. Состояние достижимо
небольшими Batch или загрузкой файла, даже если большой Replace отсечётся на входе.

- [ ] Ограничивать полный encoded response с учётом metadata и receipts.
- [ ] Проверять публикуемость нового состояния до его сохранения.
- [ ] Спроектировать компактную publication; большой config при необходимости
  получать отдельно с согласованной revision и ограниченной передачей.
- [ ] Для diagnostics считать encoded bytes, включая JSON escaping; переносить
  избыток в безопасный файловый экспорт.

Синтетическая проверка diagnostics: 640 KiB строк с кавычками дают JSON report
1311575 байт. Это доказательство ошибки бюджета, не запись реального probe.

Готово, когда любой принятый config остаётся читаемым, а допустимые ответы никогда
не рвут общий канал из-за превышения frame limit. Увеличения константы недостаточно.

### B2. P2: отделить durable admission от execution

Сейчас submit_command ждёт execute_command целиком через writer receiver при
клиентском дедлайне 15 секунд. Штатно долгие PM/KPM/firewall операции становятся
транспортной ошибкой и требуют receipt recovery.

- [ ] Возвращать Accepted после durable admission, до завершения внешних эффектов.
- [ ] Явно представить подготовку, прогресс и terminal result операции.
- [ ] Сохранить field conflict и capacity semantics; определить, какие отказы
  относятся к admission, а какие завершают уже принятую операцию.
- [ ] Проверить очередь, disconnect и рестарт между admission и первым эффектом.

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

boot::reapply_with сейчас останавливает весь apply при любом hold. Held Secret или
Cleanup способен блокировать независимые Ports после reboot, когда старых правил
уже нет.

- [ ] Использовать таблицу зависимостей из A2 для выбора разрешённых boot repairs.
- [ ] Сохранить исходные Unknown receipts и запретить повтор старых secret/cleanup
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
- [ ] **Довести writer до владения состоянием.** Выделить CommandEngine с owned
  mutable state и typed messages; читателям публиковать immutable snapshot.
  Уменьшить shared mutex/clone пути и оставить тонкий RPC adapter.
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
- [ ] **Удалить pre-1.0 importer из постоянного runtime.** Убрать его UI, startup
  ветки и legacy reads каждого snapshot. Если direct upgrade с 0.7.x требуется,
  предоставить отдельный одноразовый importer с явной совместимостью.
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

1. Исправить A1 и A2; закрыть A3–A5 регрессиями на конкретные отказы.
2. Исправить B1; разделить admission/execution и согласовать ресурсные бюджеты B2–B4.
3. Исправить provenance, export lifecycle и measurement identity C1–C3.
4. Завершить runtime reconciliation и независимое boot recovery D1–D3.
5. Удалять легаси и упрощать модели небольшими проверяемыми изменениями этапа 5.

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
