_2026-07-01_

## English

A malformed port rule in the stored config no longer discards the whole configuration (which silently disabled every hook), and selected-app UIDs now resolve by literal package match so similarly named packages are never targeted by mistake. The activator also warns on stderr when native targets exceed the 64-target backend cap, instead of silently dropping the highest-UID apps from native protection.

## Русский

Некорректное правило портов в сохранённом конфиге больше не отбрасывает всю конфигурацию (что молча отключало все хуки), а UID выбранных приложений теперь определяются по точному имени пакета, поэтому похожие имена больше не попадают в цели по ошибке. Активатор также предупреждает в stderr, когда нативных целей больше 64 (лимит бэкенда), вместо молчаливого исключения приложений с наибольшим UID из нативной защиты.
