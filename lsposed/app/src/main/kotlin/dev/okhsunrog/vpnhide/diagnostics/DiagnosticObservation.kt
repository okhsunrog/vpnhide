package dev.okhsunrog.vpnhide.diagnostics

import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.filterNotNull

/** A refresh temporarily makes readiness unknown; that is not another transition into the VPN. */
internal fun Flow<DiagnosticGate?>.routedTransitions(): Flow<DiagnosticGate> =
    filterNotNull().distinctUntilChanged().filter { it == DiagnosticGate.ROUTED }
