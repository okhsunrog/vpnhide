@file:Suppress("DEPRECATION")

package dev.okhsunrog.vpnhide.hook

import android.net.LinkProperties
import android.net.NetworkInfo
import android.os.Build
import de.robv.android.xposed.XposedHelpers

private data class FieldProbe(
    val key: String,
    val clazz: Class<*>,
    val name: String,
    val minSdk: Int = 0,
    val typeCheck: (Class<*>) -> Boolean,
)

private data class CtorProbe(
    val key: String,
    val clazz: Class<*>,
    val params: Array<Class<*>>,
)

/** Probe the private Android members required by the parcel hooks. */
internal fun runHookReflectionSmokeCheck(): List<String> {
    val broken = mutableListOf<String>()
    for (probe in FIELD_PROBES) {
        if (Build.VERSION.SDK_INT < probe.minSdk) continue
        val field =
            try {
                XposedHelpers.findField(probe.clazz, probe.name)
            } catch (_: NoSuchFieldError) {
                broken += probe.key
                continue
            }
        if (!probe.typeCheck(field.type)) broken += "${probe.key}:type=${field.type.name}"
    }
    for (probe in CTOR_PROBES) {
        try {
            probe.clazz.getDeclaredConstructor(*probe.params)
        } catch (_: NoSuchMethodException) {
            broken += probe.key
        }
    }
    return broken
}

private val FIELD_PROBES =
    listOf(
        FieldProbe("LinkProperties.mIfaceName", LinkProperties::class.java, "mIfaceName") {
            it == String::class.java
        },
        FieldProbe("LinkProperties.mRoutes", LinkProperties::class.java, "mRoutes") {
            MutableList::class.java.isAssignableFrom(it)
        },
        FieldProbe("LinkProperties.mStackedLinks", LinkProperties::class.java, "mStackedLinks") {
            MutableMap::class.java.isAssignableFrom(it)
        },
        FieldProbe("NetworkInfo.mNetworkType", NetworkInfo::class.java, "mNetworkType") {
            it == Integer.TYPE
        },
        FieldProbe("NetworkInfo.mState", NetworkInfo::class.java, "mState") {
            it == NetworkInfo.State::class.java
        },
        FieldProbe("NetworkInfo.mDetailedState", NetworkInfo::class.java, "mDetailedState") {
            it == NetworkInfo.DetailedState::class.java
        },
        FieldProbe("NetworkInfo.mIsAvailable", NetworkInfo::class.java, "mIsAvailable") {
            it == java.lang.Boolean.TYPE
        },
    )

private val CTOR_PROBES =
    listOf(
        CtorProbe(
            "LinkProperties.<init>(LinkProperties)",
            LinkProperties::class.java,
            arrayOf(LinkProperties::class.java),
        ),
        CtorProbe(
            "NetworkInfo.<init>(int,int,String,String)",
            NetworkInfo::class.java,
            arrayOf(Integer.TYPE, Integer.TYPE, String::class.java, String::class.java),
        ),
    )
