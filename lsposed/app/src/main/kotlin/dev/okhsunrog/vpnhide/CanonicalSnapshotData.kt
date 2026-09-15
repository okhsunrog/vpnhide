package dev.okhsunrog.vpnhide

/** Detach adapter-owned mutable collections before retaining a config across asynchronous phases. */
internal fun canonicalConfigSnapshot(config: CanonicalConfig): CanonicalConfig =
    config.copy(
        apps =
            config.apps.mapValues { (_, app) ->
                app.copy(
                    javaHooks = app.javaHooks?.toList(),
                    native =
                        app.native.copy(
                            overrides =
                                app.native.overrides.copy(
                                    kernel =
                                        app.native.overrides.kernel
                                            ?.toList(),
                                    zygisk =
                                        app.native.overrides.zygisk
                                            ?.toList(),
                                ),
                        ),
                    portPolicy = app.portPolicy?.let { it.copy(rules = it.rules.toList()) },
                )
            },
        settings =
            config.settings.copy(
                optionalFeatures = config.settings.optionalFeatures.toSet(),
                autoHideExcludedPackages = config.settings.autoHideExcludedPackages.toSet(),
                autoHiddenPackages = config.settings.autoHiddenPackages.toSet(),
            ),
    )

internal fun configFieldSnapshot(fields: Set<ConfigField>): Set<ConfigField> =
    fields.mapTo(linkedSetOf()) { it.copy(segments = it.segments.toList()) }
