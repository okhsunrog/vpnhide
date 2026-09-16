package dev.okhsunrog.vpnhide.hook

/** A replacement network must replay its initial properties even when values are equal. */
internal class CallbackPayloadHistory {
    private var network: Int? = null
    private val values = mutableMapOf<Int, Any>()

    fun available(source: Int): Boolean {
        if (network == source) return false
        network = source
        values.clear()
        return true
    }

    fun changed(
        kind: Int,
        value: Any,
    ): Boolean = values.put(kind, value) != value
}
