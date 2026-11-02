# Kotlin → Rust/Core: аудит владения логикой и план удаления дублирования

**Репозиторий:** okhsunrog/vpnhide. **Срез:** `045b58a439d62c2a1dbd407321d60a51ce8cb87b`, 2 ноября 2026. Только чтение исходников; без изменений проекта, запуска сборок, операций на телефоне и оценки незакоммиченной работы другого исполнителя.

## Вывод

**Да: каноническое состояние, решения о защите, проверки применимости, диагностика и восстановление должны принадлежать Core.** Сейчас в Kotlin осталась не только отрисовка: приложение собирает уже полученные от Core факты обратно в собственные модели здоровья, запускает диагностику, определяет её актуальность, выбирает рекомендации, пересчитывает политику и инициирует reconciliation. Это реальный источник лишних состояний и синхронизаций.

Но цель — **одна реализация каждого доменного правила**, а не «весь код в root». Kotlin остаётся адаптером Android/UI; переносимые чистые алгоритмы hot path могут исполняться в общей Rust-библиотеке локально, без RPC на каждый callback. Draft пользователя, навигация и реальные Android callback registrations не являются второй канонической конфигурацией.

Наибольший эффект дадут пять законченных вертикалей:

1. **Core diagnostics + единая типизированная health projection** для Dashboard, Diagnostics, Agent API и экспорта.
2. **Core policy intentions/preview/inventory** вместо Kotlin transforms, пересборки полного canonical и повторной проверки capacity.
3. **Core MaintenancePlan** для импорта/repair/update/bootstrap; отдельный provisioning с отдельным согласием.
4. **Core capture session** вместо APK-оркестрации baseline → suite → повторных root reads → ZIP.
5. **Derived Java policy + локальные чистые Rust-решения** для LSPosed; Android attachment и реальные Binder-объекты остаются адаптером.

Это части уже разрешённого цикла до первой beta, не предложение отложить RFC на несколько следующих релизов. Исследовательские gates остаются gates доказательства, а не отменой разрешения реализовывать.

## 1. Основание и фактический объём

Проверены дерево Kotlin, ключевые владельцы состояния и их вызовы, Rust owners/контракты, тестовые сценарии, оба RFC и implementation ledger. Подробно разобраны доменные узлы и границы; визуальные Compose-файлы инвентаризированы и проверены на доменные вызовы, но это не построчный UI-review.

Измерено скриптом по pinned checkout, **физические строки включая комментарии и пустые строки**:

| Область | `.kt` файлов | Строк |
|---|---:|---:|
| `lsposed/app/src/main/kotlin` | 227 | 41 696 |
| `lsposed/app/src/test/kotlin` | 123 | 17 375 |
| production `diagnostics/` | 37 | 6 724 |
| production `debug/` | 15 | 2 805 |
| production `migration/` | 2 | 467 |
| production `hook/` | 20 | 3 271 |
| production `picker/` | 14 | 3 174 |
| production `settings/` | 18 | 3 128 |
| production `statistics/` | 3 | 1 355 |

В test tree найдено **889 аннотаций `@Test`**. Это не число проверенных запусков, не число уникальных бизнес-инвариантов и не обещание удалить определённую долю тестов. `layout-fixture/LayoutHarness.kt`, Gradle Kotlin DSL и генерируемые во время сборки bindings в эти main/test числа не включены. Папочные суммы содержат экраны и adapters: их нельзя выдавать за размер будущего Rust port.

Источники решений:

- [RFC 0001: automatic GKI/KMI](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/docs/rfcs/0001-automatic-gki-kmi-selection.md).
- [RFC 0002: autonomous diagnostics](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/docs/rfcs/0002-autonomous-diagnostics.md).
- [Unified Core ledger и current-cycle scope](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/docs/notes/unified-core-implementation.md).
- [Текущая Core architecture](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/docs/core-architecture.md).

`Proposed`/`Draft` в импортированных RFC не означает отсутствие разрешения на реализацию. Ledger явно фиксирует разрешение реализовать оба RFC в текущем цикле. Вместе с тем framework fixture и kernel preflight нельзя объявить доказанными по одному документу.

## 2. Что уже принадлежит Core — не переносить повторно

Уже есть `Kotlin → core-ffi → core-client → core-rpc → core-daemon`, persistent session и generated UniFFI types; APK не исполняет `su` и не линкует privileged engine.

- `CommandEngine`: единственный writer canonical, admission, конфликты, phase execution, immutable receipts и recovery.
- `config_edit.rs`: reducer полевых изменений уже перенесён из Kotlin.
- Native startup/Prepared/pinned payloads/journal, procctl, Zygisk publication, Ports delivery — Core.
- Root observations, bounded fixed probes и безопасная выдача файлов — Core.
- Debug lease, logcat child, byte/time caps, crash restoration, retained recording до ACK — **уже Core**, не новая задача миграции.
- На этом срезе RPC **13**, kernel control **2**, private journal **6**. Не смешивать версии этих границ.

Остаточный Kotlin `ConfigCoordinator` сейчас координирует UI-intents, drafts и waiters, а не исполняет root phases. Его нужно **сужать**, а не заменять вторым конфигурационным движком под новым именем.

## 3. Конкретная карта переноса

Пути Kotlin ниже относительно `lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/`. Ссылки закреплены за тем же commit.

| Узел / функции | Что принадлежит Rust/Core | Что остаётся Android/UI; критерий удаления старого |
|---|---|---|
| `diagnostics/DiagnosticDomain.kt`: `run`, `retry`, `captureRun`, `configOperation`, `awaitSnapshot`; `DiagnosticsCache` | Admission, run identity, очереди, зависимости от операций, accepted snapshot, terminal publication. Один Core diagnostic manager. | Подписка на typed snapshot и кнопка Recheck. После переключения всех потребителей убрать process-owned diagnostic domain, не оставлять fallback owner. |
| `DiagnosticRunData.reduceDiagnosticRun`, `DiagnosticRunControlData`, `DiagnosticRunCoordinator` | Pure state machine, deadline/drain/quarantine, matching run/capture identity, late-effect fencing, сохранение last complete. | Android worker исполняет bounded команды, но не решает итог и допустимость следующего run. Портировать инварианты, не старую зависимость от чужого VPN. |
| `DiagnosticEligibilityData`, `DiagnosticContextData`, `MeasurementData`, `DiagnosticConfirmationData` | Readiness, fingerprint, applicability, полнота required plan, evidence sufficiency и cache invalidation. | Удалить self-in-tunnel/PID/external VPN key как основание HookHealth. Не переносить `owedConfirmation` буквально. |
| `DesiredDeliveryData.desiredDeliveryIdentity`, `DiagnosticImpactData.operationAffectsSelfMeasurement` | PolicyHealth из настоящих desired/delivered/process generations; Core уже знает операции. | Удалить мост `ConfigCoordinator → DiagnosticImpactObserver → local run owner`; UI pending edits остаются локальными. |
| `CheckOutcome.classifyNativeOutcome/classifyJavaOutcome`, `LayerStatus`, `DiagnosticReport.buildDiagnosticReport`, `DiagnosticPresentation`, `SituationData.situation`, `ProtectionVerdict` | Canonical outcomes/rollups/precedence/current success. Один locale-independent ответ всем каналам. | Локализованные подписи, цвета, accessibility, layout. `clean == true → HiddenByBackend` нельзя сохранить без положительного fixture control. |
| `NativeChecks.NATIVE_CHECKS`, `HookRegistry.ownedNativeHooks/expectedInstalledHooks/missingBackendHooks` | Stable vector IDs, owner/required-coverage contract, optional-feature applicability — общий registry, связанный с `data/hooks.toml` и Rust probes. | Только `checkId → R.string`. Не выводить required plan из фактически installed mask. |
| `JavaChecks.javaCheck`, `runCoreChecks`, `checks/NativeProbe` | План и классификация из Core; native probe implementation уже Rust. Убрать повторный Kotlin decode, где возможен typed binding. | Реальные `ConnectivityManager`, Java `NetworkInterface`, callback observation и JNI в Zygote-created worker. Не заменять app evidence root syscall'ами. |
| `debug/NetworkViewData.evaluateNetworkView`, consistency helpers; `diagnostics/LegacyVpnInfoData` | Pure coherence/invariant evaluation из структурированных observations. | `NetworkViewCapture`: получение Android capabilities/LP/handles/callback/PendingIntent. `NetworkViewProbeMain` остаётся вспомогательным dev probe, не production proof Zygisk. |
| `DashboardData.classifyKmodProblem`, `detect*Module`, `deriveModuleFacts`, `resolveLsposedState`; `DashboardIssues.dashboardIssues` | Capability/inventory/health/reason/action classification в одной Core projection. | `DashboardIssueRender`, `CoreDashboardText`, hero/layout — rendering. Сохранить locale-independent reason IDs из `DashboardIssueData`. |
| `DashboardData.buildNativeInstallRecommendation`, `parseKernelSeries`; `DashboardIssues.downloadArtifactFor/betterBackendIssues` | RFC1 candidates/evidence в существующем verified catalog selector. | **Удалить** manual GKI ZIP chooser, альтернативный ZIP «попробуйте другой», APK backend recommender. Legacy inventory нужен maintenance, не обычному chooser. |
| `NativeBackendData.detectNativeBackendStates/displayNativeBackend`, `NativeBootStatus`, `ModuleIntegrityData`, `KpmLoadStatus` | Selected/applied/observed/healthy и причины должны приходить typed из Core; health не парсится повторно из text. | Thin generated-type adapter допустим. Не перепутать passive payload, installed legacy module и actual live owner. |
| `DashboardData.readLsposedConfig` | Core-owned bounded platform observation: DB/schema/read failures и enabled/scope facts. Сейчас Core копирует DB+WAL/SHM, а Kotlin SQL классифицирует результат. | Если SQLite чтение временно Android-adapter, он возвращает факты, не финальную health-классификацию. Сохранять DB consistency, version/ROM uncertainty. |
| `settings/FilesystemHidingData.resolveFilesystemHidingState`, `filesystemNativeBackend`, `currentZygiskHookStatus` | FeatureHealth: desired/effective, loaded code generation, required restart/reboot, setup failures. | Switch/dialog/текст по typed requiredAction. Boot ID без process generation недостаточен для новой Zygisk policy. |
| `RootSnapshotCache`, `DashboardCache`, `TargetsCache`, `RoutingGateCache`, root-derived части `ObservationCoordinator` | Typed Core observations и согласованный publication; raw sections не второй semantic API. | Один reconnect/epoch-fenced UI replica; local inventory/icon loads могут сохранить generic cache. Удалять не весь `StateCache`, а root-domain joins/invalidations, ставшие ненужными. |
| `CanonicalEditData.applyCanonicalEdit/applyCanonicalMutation`, `StorageConfig.buildCanonicalConfig*`, `CanonicalConfigData` parsing | Один Rust reducer/validator/normalizer; typed intent и shared unprivileged preview вместо второго Kotlin reducer. | Draft text, unsaved values, UI conflict protection/rebase/waiters. Нет необходимости RPC на каждый введённый символ. |
| `picker/AppPickerData.nativeTargetCapacityUsage/nativeCapacityIncreaseViolation` | Тот же family-aware policy projection, что commit; Kotlin прямо зеркалит `activator::native_target_set` и содержит 160/1. | Preview capacity/contributors от Core или shared pure Rust. Удалить hardcoded counter после parity. |
| `AppPickerData.resolveAutoHiddenPackages/applyAutoHiddenPackages/updateManualHiddenPackages`, `ConfigChannels.reconcileAutoHiddenPackages` | Auto-hide policy, manual/automatic provenance, exclusions, partial-inventory rules и reconciliation trigger — Core. | PM service declarations/labels/icons — evidence adapter. Name heuristic тоже domain rule; locale provenance учитывать явно. |
| `picker/PackageInventoryData`, `UserProfileData`, `ShellUtils.parsePackageUidMap` | Typed inventory/profile completeness/UID resolution; Rust уже собирает inventory и имеет `PackageUidMap`. | Labels/icons/profile names, search/sort. Failed profile ≠ complete-empty; нельзя удалять отсутствующий пакет после partial scan. |
| `ShellUtils.ensureSelfInTargets/buildCanonicalSelfUpdate/writeStartupCanonical`, `StartupCoordinator` | Self-target preparation, migration decision, PM readiness, startup repair trigger — Core lifecycle перед worker creation. | UI readiness/subscription. Убрать обязательные startup suite и once-per-APK `RepairActivation`, когда Core boot/recovery обеспечивает их смысл. |
| `migration/LegacyConfigImport`, `CanonicalImportData` | Parse/resolve/merge/replace/strict schema validation, cleanup eligibility, preservation и plan-bound execution. | SAF input, пользовательский выбор merge/replace/discard, progress. Сохранить isolated 0.x/1.x adapters; не сливать tolerant hook reader и strict import parser. |
| `AgentControl.setAppProtection/set*Hooks/setPortPolicy/setAutoHideSettings/setManualHiddenPackages/importCanonicalConfig` | Typed domain methods, validation, patch semantics, reports/statistics. Agent и UI используют одну логику. | `AgentControlDispatcher` UI scenarios/navigation остаются. `AgentControlBridge` — непривилегированный gateway, не root HTTP сервер. |
| `statistics/StatisticsData.buildStatisticsState`, protocol parsing, `ProbeStats` aggregation/diff | Backend identity, unsigned counters, reset/generation attribution, truncation, baseline/delta и rollups — typed Core facts. | Formatting, sort, selected rows. Избавиться от root-derived `StatisticsCache` authority, не от display subscription. |
| `debug/DebugExport.buildDebugState/exportDebug`, `ExportRecommendationData.recommendExport` | Capture plan/session ID, lease sequencing, baseline, fresh run, snapshot join, error policy и attachment recommendation. | Explicit selections, SAF/share и file delivery. ZIP может исполняться unprivileged Rust helper; root нужен только privileged acquisition. |
| `debug/LogcatRecorder`, `JavaArtifactExport`, `KernelImageExport` | Не повторять существующий Core lifecycle. Общая capture/export policy может убрать APK packaging joins. | Start/Stop UI, progress, delivery/ACK bridge. Java artifacts уже получаются `CoreProbe.JavaHookArtifacts`. |
| `UpdateChecker` | Portable version/release/compatibility logic и общий update domain для MaintenancePlan. | HTTP может остаться Android/unprivileged; WorkManager/permissions/notifications/local delivery bookkeeping остаются. Не заводить root internet stack без нужды. |
| `hook/SystemServerConfigData.javaHookMasksByAppId`, `SystemServerConfigCache` | Core-derived generation-bound Java policy либо один shared Rust parser/projector; union shared appId, hidden/observer rules. | Безопасное чтение immutable derived snapshot, FileObserver и actual runtime binding. Derived policy не новый canonical engine. |
| `hook/CallbackLifecycleData`, `NetworkViewResolverData.isUsableCover`, pure часть `VisibleNetworkResolver.coverFor` | Возможен перенос в локальную Rust library: scalar input → typed decision, без remote hot-path calls. | Reflection, Binder identity, Network/Parcel objects, callbacks/Messenger/PendingIntent, actual registration state, attachment и telemetry capture. |

### Важные вызовы домена из экранов

Не ограничиваться файлами `*Data.kt`: `AppPickerScreen` собирает canonical и применяет auto-hide; `HiddenAppsSettingsScreen` передаёт `editor.save { applyAutoHiddenPackages(...) }`; `SettingsScreen` переключает auto-hide через `CanonicalMutation.transform`. После общего intent API эти экраны должны посылать **намерения**, а не реконструировать итоговый canonical. `FullResetDialog` сохраняет подтверждение пользователя и отправку typed reset, но blockers и исполнимость определяет Core.

## 4. Целевой контракт: один owner, не один гигантский объект

### 4.1. Чтение

Расширять существующий `CorePublication`, не добавлять параллельный JSON facade:

- `BackendState`: catalog availability, eligible candidate/evidence, selected, applied, observed owner, loaded code identity и process exposure — разные поля.
- `PolicyHealth`: desired revision, delivery evidence по слоям, effective generations, pending requirements, held resources.
- `DiagnosticState`: active session, last attempt, last complete report, fingerprint/applicability, cleanup state; per-vector typed evidence/outcome.
- `ProtectionAssessment`: единая machine-readable severity/reason/action модель, из которой UI делает локализованный текст.
- `Inventory/Feature/Statistics` typed snapshots с provenance/freshness; большие forensic data по bounded handle/artifact, не в безразмерном publication.
- `MaintenancePlan/Progress`: identity/digest, prerequisites, destructive steps, consent scope, durable receipts и reconciliation.

Граница snapshot должна сохранять Core epoch, publication/config revision, boot identity и соответствующие loaded/process generations. Не склеивать независимо прочитанные `.value` разных ревизий. Сохранить правило «publication до завершения waiter» и отделение retained history от current evidence.

### 4.2. Команды и preview

Команды условно: `Recheck`, `BeginCapture`, typed role/hook/port/manual-hidden intentions, `PreviewEdit`, `PlanImport/Maintenance`, `ExecutePlan`. Имена здесь проектные, не утверждение о существующем RPC.

- Реальные mutations продолжают единственный `CommandEngine`, current revision, durable operation ID и recovery-by-ID; lost reply не разрешает replay.
- Preview использует тот же reducer/validator; не persistent second config store. Live-dependent capacity требует inventory/catalog generation, устаревший preview не гарантирует будущий commit.
- UI drafts не загружаются в root на каждый символ. Общий pure Rust reducer можно вынести из daemon-only модуля в непривилегированную библиотеку; сам daemon APK не линкуется.
- Side-effectful domains не надо выполнять внутри одного долгого mutex: scheduling/resources ограничены, writer admission интегрирован, RPC readable во время работы.

### 4.3. Безопасность

`AgentControlBridge` намеренно пишет debug bearer token в logcat для release MCP. Это существующий opt-in loopback workflow. **Не переносить транспорт и credential в root** и не считать этот токен разрешением на новый привилегированный endpoint. Domain API — через существующий authenticated Core канал, gateway непривилегированный.

`core-daemon/export.rs` использует pinned descriptors, descriptor-relative no-follow открытие, проверку regular files и не переоткрывает root destination по уже выданному приложению пути. Эти защиты должны пережить перенос ZIP/capture. Не добавлять caller-supplied root paths/commands.

## 5. RFC: что переносить, а что заменить по смыслу

### RFC 0001

- Единственный selector — `payloads/selection` + projections + `Prepared` + journal; Kotlin KMI recommendations удаляются.
- Candidate set из **verified manifest**, hints/evidence отдельно. Ambiguity не разрешается «первый прошедший» и не уходит пользователю выбором ZIP.
- Preflight — исполнение kernel loader, не пассивная ABI-проверка. Перед вызовом нужен **hash-bound protocol capability** финального `.ko`: неизвестный параметр модуль может проигнорировать и выполнить обычный init.
- Negative-init sentinel, отсутствие VPNHide effects, cleanup и distinct bounded preflight/real-load budgets проверяются отдельно. Sentinel не доказывает private ABI/hook safety.
- Current artifact/generation, unknown effects, prior Zygisk exposure и compatible ownership guards сохраняются. Не загружать другой backend поверх неопределённого результата.

### RFC 0002

Не переносить текущую suite «как есть» в Rust: её ключи и positive-control предпосылки изменяются.

1. Core создаёт bounded native TUN fixture; framework fixture отдельна. TUN не превращается автоматически в Android VPN network.
2. Реальный `:diagnostics` Android worker через Zygote и нормальный UID/SELinux/Zygisk, не root `app_process` с `setuid`.
3. RAW/control фаза изолирована по process/session/registration, **не отключением защиты всему UID**, который разделяется с UI. Core проверяет worker capability/nonce и process identity.
4. Required plan из supported contract + enabled features. Отсутствующий required hook/fixture stimulus не сокращает план до зелёного.
5. Положительный RAW stimulus, protected differential, **обязательное сохранение обычной non-VPN сети**, consistency и cleanup. Пустой ответ/отсутствие VPN/SELinux denial не автоматически HookHealth PASS.
6. Отдельно HookHealth и PolicyHealth. Fresh worker PASS не доказывает, что уже запущенные игры потребили новое поколение policy.
7. Fingerprint включает actual loaded code, boot, system_server lifecycle, probe/fixture/schema/required-plan, user/UID и релевантную среду, включая **Connectivity APEX version**. PID worker — provenance, не долгоживущий key; внешний VPN netId исключается.
8. Нет expensive suite на каждом APK-open/VPN toggle. Core RAM result допустим; Core epoch/RAM loss не маскировать APK last-good snapshot.
9. Java callback/framework feasibility остаётся device-gated hypothesis. Не объявлять полный RFC исполненным по native-only PASS.

## 6. Порядок выполнения в существующих stages 4–7

| Вертикаль | Встраивание | Доказательство завершения / удаление |
|---|---|---|
| A. Selector/loader generation + RFC1 | Stage 4 | Verified active catalog → full per-family policy → frozen Prepared → journaled effects; own Zygisk loader handshake, generation observer, bounded safe fallback. Никакого manual KMI chooser. |
| B. Typed health/report | Stage 5, параллельно независимым A tests | Один ответ Dashboard/Agent/export/settings; parsed raw sections больше не выбирают backend/health. RPC skew безопасно блокирует, а не включает Kotlin oracle. |
| C. Autonomous diagnostic vertical | Stage 5 + RFC2 | Сначала Java callback feasibility и worker launch spike вместе с native fixture; затем state machine/report/cache/boot trigger. Удаляются local diagnostic owner и external routing admission. |
| D. Intents/inventory/preview | Stage 5 и вход stage 6 | Shared reducer/validator; Core auto-hide/self-target readiness; partial profile и capacity parity. Удаляются Kotlin canonical transforms и mirrors, не UI drafts. |
| E. Capture/export | Stage 5/6 | Один capture identity, baseline до его run, accepted report, cleanup/restoration при crash, retained delivery/ACK. Удаляются многошаговые APK root joins. |
| F. Maintenance/import/update | Stage 6 | Один typed plan/executor, sources сохранены до verified replacement; active/staged generations не смешаны; interrupted updates reconcile. Удаляются app-owned migration/cleanup eligibility. |
| G. Runtime provisioning | Stage 7 | Отдельный verified tool/fork tuple, KALLSYMS/ALL checks, original backup, slot/OTA/recovery tests, plan-bound explicit boot consent. Отсутствие KPM не запускает автопрошивку. |
| H. Hook-local pure rules | В пределах тех же policy/diagnostics вертикалей | Derived policy или shared Rust local engine, generation/read failure/last-good behavior, callback parity и Android device evidence. Нет RPC на горячем Binder path. |

Initial absent/incompatible Core не может установить себя через отсутствующий RPC: manager/bootstrap adapter остаётся явным. Repair не удаляет journal ради разблокировки. Файловый rollback не равен отмене resident hooks или boot image changes.

## 7. Что удалять, а не переводить строка в строку

- APK manual backend/GKI download selector и совет перебора ZIP.
- `StartupCoordinator.ensureInitialCaches → DiagnosticsCache.run` и `ensureProtectionCacheAfterRootSnapshot → RepairActivation` как обязательные APK lifecycle эффекты после Core-owned замены.
- External-VPN/self-routing gate для synthetic HookHealth, PID/VPN-session keyed auto-confirmation и соответствующие «включите VPN/добавьте VPN Hide в туннель» UX-ветки.
- Второй reducer canonical edits, app-owned transforms, reconstruction canonical из targets/raw snapshot. Использовать существующий Rust reducer, расширяя его точными intentions.
- Kotlin native capacity mirror, duplicated raw protocol/inventory parsers там, где Core отдаёт typed facts.
- Root-domain observer caches/joins, от которых больше нет потребителей после typed publication. Generic cache для icons/локальных Android наблюдений может остаться.
- Несколько derived report/health builders для экрана, Agent и ZIP; общий Core report + разные renderers.
- Obsolete bootstrap/import wrappers после MaintenancePlan. **Не удалять** поддержку прямого 0.x/1.x upgrade вместе с wrappers.
- Typed wrapper-to-wrapper модели без added UI meaning можно заменить generated types; но transport lifecycle/epoch fencing не «лишний boilerplate».

## 8. Проверки паритета и критерий удаления

До удаления старого владельца для каждой вертикали должны существовать:

1. Явный перечень сохранённых инвариантов и намеренно изменённых RFC-семантик.
2. Language-neutral fixture vectors либо Rust behavior tests на новом authoritative owner. Не двойной production owner «для надёжности».
3. Contract tests Kotlin↔UniFFI↔RPC; unknown enum/status, bounds, skew, stale generation, epoch reset.
4. Негативные сценарии, дающие действительно разные результаты: missing required hook, stale heartbeat, missing baseline, failed cleanup, failed profile scan, post-rename uncertainty, reply loss и interrupted run.
5. Проверка всех потребителей: UI, Agent, export и settings получают одинаковые reason/current-success для одной ревизии; локализация не меняет domain outcome.
6. Для Android-dependent поведения — device acceptance: обычный app process, system_server restart, existing target processes, background launch, реальный callback lifecycle, normal-network preservation, 32-bit там, где поддерживается.
7. Поиск оставшихся production call sites старого owner; затем удаление owner, его triggers, моделей/парсеров без потребителей и **дублирующих**, но не уникальных тестов.

Архитектурный аудит не запускал тесты и не подтверждает устройство. Ни заявленные в ledger прошлые pass counts, ни host fixtures не заменяют приёмку нового vertical slice.

## 9. Ценность Kotlin-тестов: переносить поведение, не число файлов

Большое число тестов здесь объясняется в том числе количеством **дублирующих доменных моделей**. Наиболее полезное сокращение — удалить второй production implementation и его вторую domain suite. Короткий очевидный тест не обязательно бесполезен: `Unknown != PASS`, «сохранено, но не применено» и «новая epoch с прежней revision» — именно те маленькие различия, которые предотвращают опасную ложную зелень.

### 9.1. Конкретные кандидаты на объединение/удаление

Пути ниже относительно `lsposed/app/src/test/kotlin/dev/okhsunrog/vpnhide/`. Проверены тела тестов. История известных багов приведена по комментариям репозитория, не по независимо восстановленному Git history.

| Тест | Что фактически проверяет | Решение и замена |
|---|---|---|
| `NetworkViewCheckTest.kt:35` — `the verdict is a three-way distinction` | Повторяет те же четыре input/output случая, что три теста выше: violation wins, clean incomplete, clean complete. | Оставить одну явно названную таблицу с четырьмя случаями; убрать второй экземпляр. **Не трогать** callback-history и handle-isolation ниже. |
| `DetectModuleMismatchesTest.kt:59` — `module kind and versions preserved in result` | Core `0.5.0 → 0.6.2` уже проверен полным data-class equality в `different base version produces a mismatch per module` на строке 41. | Удалить/слить отдельный дубликат, сохранив null/version/presence cases. Позже пересмотреть mismatch policy для independently compatible unified payloads. |
| `HookCheckCoverageTest.kt:43` — `the unchecked exemption list stays honest` | Пересечение локальной константы `emptySet()` с checked registry всегда пустое. Сейчас не ловит production regression. | Удалить отдельную тавтологию либо объединить с реальной generator validation при появлении exemptions. **Сохранить** `every kernel hook is covered by a diagnostic check`. |
| `BundleSchemaGoldenTest.kt:97` — `the golden carries the current schema number` | Частный случай полного сравнения сериализованного sample с golden на строке 62; sample использует current schema default. | Необязательное объединение в основной golden с понятной диагностикой. Это не повод удалять schema/version policy и discriminator guard. |
| `AgentControlDispatcherTest.functionsJsonRoundTrips`: `description.contains("baseline")` | Закрепляет английское слово в описании, а не обязательность argument/schema. | Заменить проверкой required baseline argument/advertised schema. Остальные envelope/API assertions сохраняются. |
| `DebugShellSnapshotTest`: `debug probe reads the backend state paths the app shares`; `counter probe captures hook status without enumerating packages` | Читает Rust source через `contains` и substring. Комментарий/мёртвый код может дать PASS, harmless refactor — FAIL. | Перенести в исполняемые Rust collector/output + fake-command call assertions; затем убрать source-text checks. Инвариант «counter probe не делает expensive inventory» сохранить. |
| `RootSnapshotCacheTest`: `the snapshot probe reports the probe identity for the honest-render gate` | Проверяет написание `snapshot_shell_uid` и `sys::uid()`, не фактический результат. | Rust collector test emitted identity + error semantics; затем удалить слабую текстовую проверку. |

Уже есть основа более сильных замен: `crates/core-daemon/src/device_probes/tests.rs:276` — `root_snapshot_emits_every_section_the_app_requires`; `:298` — `debug_snapshot_covers_every_backend_state_path`. Они исполняют collectors, но **не заменяют автоматически** все performance/no-inventory assertions: туда ещё нужны command spies.

### 9.2. Тесты, которые переезжают вместе с domain owner

| Kotlin suites | Новый owner | Что обязательно сохранить |
|---|---|---|
| `DiagnosticRunDataTest`, `DiagnosticRunCoordinatorTest`, `DiagnosticDomainTest`, `DiagnosticSnapshotTest` | Core diagnostic reducer + manager integration | Late events, run/capture identity, waiter detach не cancel, deadlines/drain/quarantine, retained partial evidence, publication-before-completion. |
| `ClassifyNativeOutcomeTest`, `MeasurementDataTest`, `DesiredDeliveryDataTest`, `LayerStatusTest`, `DiagnosticReportTest`, `ProtectionVerdictTest` | Core evidence/health | Unknown≠PASS; отдельно SELinux; desired/delivered/measured; stale/current и missing required evidence. |
| `DashboardIssuesTest`, `ClassifyKmodProblemTest`, `ClassifyKpmProblemTest`, `ModuleIntegrityDataTest`, `NativeBootStatusTest`, `FilesystemHidingDataTest` | Core health/selection/maintenance | Typed severity/reason/action precedence; Kotlin оставляет renderer и reason-ID contract. |
| `LegacyConfigImportTest`, `CanonicalImportDataTest`, domain cases `StorageConfigTest` | Isolated Core 0.x/1.x adapters | Future/corrupt schema refusal, source completeness, unresolved UID, прямой upgrade, сохранение manual roles/self policy. |
| Domain cases `AppPickerDataTest`, `HiddenAppsDataTest`, `SettingsDataTest`, `PackageInventoryDataTest` | Core intentions/inventory/auto-hide | Невидимые пакеты сохраняются; partial profile не complete-empty; manual и automatic provenance отдельно. UI filtering/drafts остаются Kotlin. |
| `NativeCapacityDataTest` | Existing activator projection + typed preview | UID/profile/shared-appId/reserved-self/capacity atomically refuse, не truncate. UI должен позволить уменьшить уже переполненный draft. |
| `NetworkViewDataTest`, classifier parts `NetworkViewCheckTest` | Rust evaluator | Sync/callback consistency, accumulated leak и per-handle provenance. Android capture остаётся boundary-tested. |
| `StatisticsDataTest`, rollup cases `ProbeStatsTest` | Rust statistics projection | Unsigned counters, reset, truncation, backend/session identity. Formatting остаётся локальным. |
| Portable cases `UpdateCheckerTest` | Shared unprivileged Rust policy library | Defined version comparison semantics; не случайно заменить dotted-version ordering иной SemVer политикой. |
| Pure cases `hook/SystemServerConfigDataTest`, `CallbackLifecycleDataTest`, `NetworkViewResolverDataTest` | Shared local Rust policy engine / Core-derived policy | Shared appId union; self-exception; cover fallback only when API absent, not null/error; correct callback transitions. |

Capacity уже проверяется Rust: `crates/activator/src/tests.rs:733` `a_target_set_at_capacity_projects_whole`; `:741` `an_over_capacity_target_set_is_refused_whole_never_truncated`; `:755` `capacity_counts_uids_across_profiles_skips_platform_uids_and_reserves_one_self_slot`. Свести authoritative coverage туда, оставив Kotlin preview/refusal presentation contract, а не два policy counters.

### 9.3. Исторически полезные тесты, чьи ожидания устарели после RFC

Это **не бессмысленные тесты**: они закрепляют прежний продукт. Их нельзя blindly портировать.

- `BuildNativeInstallRecommendationTest:78/90`: `no KMI 5_10 returns ambiguous android12 plus android13 alternative`, `no KMI 5_15 returns ambiguous android13 plus android14 alternative`; `ClassifyKmodProblemTest:110`: `ambiguous series suggests the other candidate`. Удалить ожидание manual ZIP workaround; вместо него Core tests manifest-derived candidates, ambiguity, no first-passing selection. Сохранить reporter regression **Android 15 ROM на android12-5.10 kernel** как доказательство, что OS version не KMI authority.
- `ClassifyJavaOutcomeTest:22`: `clean app view is hidden by the backend`. В RFC2 `true` без RAW stimulus/preservation/session consistency/cleanup недостаточно. Новые тесты должны ловить ложный PASS при исходно пустом ответе.
- `LayerStatusTest:103/131`: optional filesystem vectors owned only after hook installed. Новый required plan берёт supported contract + enabled features; enabled required hook, не установившийся в runtime, должен дать coverage gap. Disabled feature — корректный NotApplicable, не то же самое.
- `DiagnosticDomainTest`: `a tunnel re-established while the app was away is confirmed once on return, without an edge to detect`. Вместо повторного self-test от external VPN — тест **отсутствия** rerun на VPN/UI lifecycle при неизменном HookHealth fingerprint; relevant loaded-code changes запускают bounded run.

### 9.4. Маленькие, но важные тесты, которые не стоит вычищать

- `ConfigCoordinatorTest:136/179`: потерянный ответ/secret uncertainty восстанавливаются запросом того же durable ID, **без replay**. Server tests не заменяют client-boundary tests.
- `CanonicalEditorDataTest:23/33/42`: пользователь меняет/reverts draft во время save; ACK относится только к submitted revision. Это настоящий UI state, законно Kotlin.
- `CanonicalEditDataTest:72`: mutable collections копируются, secret не попадает в description. Rust ownership не защищает автоматически Kotlin/FFI objects.
- `AgentMutationDataTest`: Persist Confirmed + Native FailedKnown ⇒ `changed=true`, `ok=false`; нельзя заявить rollback.
- `CoreSessionDataTest`: connection/epoch/request watermark fencing. `CoreInputJoinDataTest`: новая epoch с той же config revision не разрешает старый evidence join. После удаления split join заменить его цель typed-publication test.
- `HookPackageBoundaryTest`: Kotlin compiler допускает app-only dependency в system_server; guard предотвращает реальный crash. Заменить можно жёсткой module isolation, не обычным Rust unit test.
- `NativeProbeContractTest`: malformed/future native wire не превращается в успех.
- `BundleSchemaGoldenTest`: внешний формат debug bundle и stable discriminators. Перенести к владельцу сериализации + оставить узкий consumer contract.
- `AtomicTelemetryTest`: rename сохраняет старый целый snapshot читателю; failed rename не уничтожает прежний файл.
- `PartialHookGapTest`: комментарии фиксируют MediaTek 4.14/Clang CFI missing-hook regression; сохранить сценарий с новой authority model.
- `NetworkViewCheckTest:48/57`: поздний чистый callback не стирает раннюю утечку; факты разных handles не создают ложную полную пару.

### 9.5. Правило для каждого удаляемого теста

В migration checklist у удаления должен быть один из четырёх мотивов:

1. **Устаревшее поведение** → конкретный новый RFC acceptance case.
2. **Точный дубликат** → имя оставшегося теста.
3. **Implementation-detail assertion** → более сильный behavioral replacement.
4. **Удалённая реализация** → новый owner и перенесённые parity fixtures.

На переходе общие vectors временно проверяют обе реализации. После удаления Kotlin implementation его duplicate domain suite удаляется; Kotlin сохраняет реальные UI/Android/FFI obligations. Нельзя сохранять второй canonical engine только для двух зелёных suites, удалять failures ради зелёного CI или назначать план «сократить тесты на N процентов».

## 10. Исходники для проверки выводов

Все ссылки immutable; номера строк относятся только к этому срезу.
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/diagnostics/DiagnosticDomain.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/diagnostics/DiagnosticDomain.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/diagnostics/DiagnosticRunData.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/diagnostics/DiagnosticRunData.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/diagnostics/DiagnosticContextData.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/diagnostics/DiagnosticContextData.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/diagnostics/DiagnosticConfirmationData.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/diagnostics/DiagnosticConfirmationData.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/diagnostics/CheckOutcome.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/diagnostics/CheckOutcome.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/diagnostics/DiagnosticReport.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/diagnostics/DiagnosticReport.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/diagnostics/DesiredDeliveryData.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/diagnostics/DesiredDeliveryData.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/diagnostics/ExportRecommendationData.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/diagnostics/ExportRecommendationData.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/startup/StartupCoordinator.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/startup/StartupCoordinator.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/CanonicalEditData.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/CanonicalEditData.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/ConfigCoordinator.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/ConfigCoordinator.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/RootSnapshotCache.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/RootSnapshotCache.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/DashboardData.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/DashboardData.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/DashboardIssues.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/DashboardIssues.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/HookRegistry.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/HookRegistry.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/picker/AppPickerData.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/picker/AppPickerData.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/picker/TargetsCache.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/picker/TargetsCache.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/settings/FilesystemHidingData.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/settings/FilesystemHidingData.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/debug/DebugExport.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/debug/DebugExport.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/debug/LogcatRecorder.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/debug/LogcatRecorder.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/debug/NetworkViewData.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/debug/NetworkViewData.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/AgentControl.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/AgentControl.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/AgentControlBridge.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/AgentControlBridge.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/migration/LegacyConfigImport.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/migration/LegacyConfigImport.kt)
- [`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/hook/SystemServerConfigCache.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/hook/SystemServerConfigCache.kt)
- [`lsposed/app/src/test/kotlin/dev/okhsunrog/vpnhide/NetworkViewCheckTest.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/test/kotlin/dev/okhsunrog/vpnhide/NetworkViewCheckTest.kt)
- [`lsposed/app/src/test/kotlin/dev/okhsunrog/vpnhide/DetectModuleMismatchesTest.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/test/kotlin/dev/okhsunrog/vpnhide/DetectModuleMismatchesTest.kt)
- [`lsposed/app/src/test/kotlin/dev/okhsunrog/vpnhide/HookCheckCoverageTest.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/test/kotlin/dev/okhsunrog/vpnhide/HookCheckCoverageTest.kt)
- [`lsposed/app/src/test/kotlin/dev/okhsunrog/vpnhide/DebugShellSnapshotTest.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/test/kotlin/dev/okhsunrog/vpnhide/DebugShellSnapshotTest.kt)
- [`lsposed/app/src/test/kotlin/dev/okhsunrog/vpnhide/RootSnapshotCacheTest.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/test/kotlin/dev/okhsunrog/vpnhide/RootSnapshotCacheTest.kt)
- [`lsposed/app/src/test/kotlin/dev/okhsunrog/vpnhide/BuildNativeInstallRecommendationTest.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/test/kotlin/dev/okhsunrog/vpnhide/BuildNativeInstallRecommendationTest.kt)
- [`lsposed/app/src/test/kotlin/dev/okhsunrog/vpnhide/ClassifyJavaOutcomeTest.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/test/kotlin/dev/okhsunrog/vpnhide/ClassifyJavaOutcomeTest.kt)
- [`lsposed/app/src/test/kotlin/dev/okhsunrog/vpnhide/LayerStatusTest.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/test/kotlin/dev/okhsunrog/vpnhide/LayerStatusTest.kt)
- [`lsposed/app/src/test/kotlin/dev/okhsunrog/vpnhide/HookPackageBoundaryTest.kt`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/lsposed/app/src/test/kotlin/dev/okhsunrog/vpnhide/HookPackageBoundaryTest.kt)
- [`crates/core-daemon/src/config_edit.rs`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/crates/core-daemon/src/config_edit.rs)
- [`crates/core-daemon/src/export.rs`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/crates/core-daemon/src/export.rs)
- [`crates/core-daemon/src/payloads/selection.rs`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/crates/core-daemon/src/payloads/selection.rs)
- [`crates/core-daemon/src/payloads/projection.rs`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/crates/core-daemon/src/payloads/projection.rs)
- [`crates/core-daemon/src/device_probes/tests.rs`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/crates/core-daemon/src/device_probes/tests.rs)
- [`crates/activator/src/model.rs`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/crates/activator/src/model.rs)
- [`crates/activator/src/tests.rs`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/crates/activator/src/tests.rs)
- [`crates/core-rpc/src/lib.rs`](https://github.com/okhsunrog/vpnhide/blob/045b58a439d62c2a1dbd407321d60a51ce8cb87b/crates/core-rpc/src/lib.rs)

**Итоговый критерий:** APK можно закрыть, открыть заново и сменить язык — это не должно пересоздать доменное состояние защиты или запустить дорогую диагностику. UI, Agent и архив видят один Core result; Android adapters поставляют факты, не конкурирующий verdict.
