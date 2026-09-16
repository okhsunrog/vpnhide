package dev.okhsunrog.vpnhide.hook

internal enum class CallbackEventKind { Available, Changed, Losing, Lost }

internal enum class CallbackDelivery { Suppress, Forward, Available, Lost }

internal data class CallbackTransition(
    val held: Int?,
    val delivery: CallbackDelivery,
    val network: Int? = held,
)

/** A registration has one visible best network. Losing the VPN is not losing its cover. */
internal fun transitionCallback(
    held: Int?,
    visible: Int?,
    event: CallbackEventKind,
): CallbackTransition =
    when {
        visible == null && held != null -> {
            CallbackTransition(null, CallbackDelivery.Lost, held)
        }

        visible == null -> {
            CallbackTransition(null, CallbackDelivery.Suppress)
        }

        visible != held -> {
            CallbackTransition(visible, CallbackDelivery.Available)
        }

        event == CallbackEventKind.Lost || event == CallbackEventKind.Losing || event == CallbackEventKind.Available -> {
            CallbackTransition(held, CallbackDelivery.Suppress)
        }

        else -> {
            CallbackTransition(held, CallbackDelivery.Forward)
        }
    }

internal fun isPassiveNetworkRequest(type: String?): Boolean = type == "LISTEN"

/** Physical delivery follows the same visible registration history as rewritten VPN events. */
internal fun transitionPhysicalCallback(
    held: Int?,
    source: Int,
    event: CallbackEventKind,
): CallbackTransition =
    when {
        event == CallbackEventKind.Available -> {
            CallbackTransition(source, if (held == source) CallbackDelivery.Suppress else CallbackDelivery.Forward)
        }

        held != source -> {
            CallbackTransition(held, CallbackDelivery.Suppress)
        }

        event == CallbackEventKind.Lost -> {
            CallbackTransition(null, CallbackDelivery.Forward, source)
        }

        else -> {
            CallbackTransition(held, CallbackDelivery.Forward)
        }
    }
