_2026-04-19_

## English

Dashboard now shows a consistent version string for all modules. Kernel-module, Zygisk and Ports module cards used to display the Magisk-style 'vX.Y.Z' from their module.prop, while the LSPosed hook module card showed the Android-style 'X.Y.Z' from the APK's versionName — on the same screen, for the same version number. The 'v' prefix is now stripped at parse time so every card reads 'X.Y.Z' (or 'X.Y.Z-N-gSHA' for dev builds).

## Русский

На панели теперь одинаковый формат версий для всех модулей. Раньше карточки модулей ядра, Zygisk и скрытия портов показывали Magisk-стиль 'vX.Y.Z' из module.prop, а карточка LSPosed-хука — Android-стиль 'X.Y.Z' из versionName APK — на одном экране, для одной и той же версии. Префикс 'v' теперь снимается при парсинге, так что везде показывается 'X.Y.Z' (или 'X.Y.Z-N-gSHA' для dev-сборок).
