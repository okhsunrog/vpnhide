# Configuration migration boundary

Support direct upgrades from both 0.x and 1.x. Do not remove an adapter merely
because intermediate releases have existed for a long time.

- `lsposed/.../migration/LegacyConfigImport.kt` owns 0.x source inventory,
  package-role conversion, profile-aware observer UID mapping and import commands.
  Its formats come from the restored importer (`a2a389af`), not a new guessed
  schema. Derived Java UID files are cleanup inputs, not a competing source of
  user intent; package lists are authoritative.
- `migration/CanonicalImportData.kt` validates explicit 1.x/current JSON imports
  before the tolerant LSPosed reader can normalize invalid shapes away. It accepts
  schema 1 boolean/list/native-object roles and rejects unknown versions and
  malformed present fields. No filesystem effects occur during parsing.
- `core-daemon/src/config_file/codec.rs` is the pure disk adapter. Schema 1 is
  also the current 2.x LSPosed contract; persistence remains in `config_file.rs`.
  The command engine consumes normalized typed config, not legacy formats.

Both import routes use ordinary typed core commands and the same candidate
validator. Persist precedes cleanup. A failed write leaves source files untouched;
uncertain effects are recovered by operation ID, never blindly repeated. Once a
successful import removes fully interpreted sources, it is complete. If sources
remain, repeated merge is idempotent. Unresolved observer UIDs, invalid lines, incomplete snapshots or unreadable sources
prevent automatic deletion; explicit Discard is still available. Missing config
permits bootstrap; an existing blank, malformed or unsupported file does not.

Representative fixtures under `fixtures/migrations/` cover pre-1 package/UID
sources and schema-1 roles. Host tests cover atomic roundtrip, failed Persist and
restart/deduplication without repeated cleanup. JVM tests cover direct 0.x/1.x to
current conversion, repeated merge, corrupt/future JSON and retained partial
sources. These tests do not establish Android filesystem labels or real upgrade
installation behavior; those remain device acceptance checks.
