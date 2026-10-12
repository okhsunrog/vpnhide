package dev.okhsunrog.vpnhide.settings

import android.content.Context
import android.content.res.Configuration
import androidx.core.content.ContextCompat
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow

/** Presentation-only input: no Activity retained and no observation invalidated. */
internal object AppLanguageContext {
    private val current = MutableStateFlow<Context?>(null)
    val context = current.asStateFlow()

    fun update(activity: Context) {
        val configuration = activity.resources.configuration
        if (current.value
                ?.resources
                ?.configuration
                ?.locales == configuration.locales
        ) {
            return
        }
        current.value = activity.applicationContext.createConfigurationContext(Configuration(configuration))
    }

    fun localized(context: Context): Context = current.value ?: ContextCompat.getContextForLanguage(context)
}
