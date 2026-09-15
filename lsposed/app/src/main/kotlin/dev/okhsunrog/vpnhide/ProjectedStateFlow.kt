package dev.okhsunrog.vpnhide

import kotlinx.coroutines.ExperimentalForInheritanceCoroutinesApi
import kotlinx.coroutines.flow.FlowCollector
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map

/** All synchronous projections read the same immutable source, without another publishing job. */
@OptIn(ExperimentalForInheritanceCoroutinesApi::class)
internal class ProjectedStateFlow<T, R>(
    private val source: StateFlow<T>,
    private val project: (T) -> R,
) : StateFlow<R> {
    override val value: R get() = project(source.value)
    override val replayCache: List<R> get() = listOf(value)

    override suspend fun collect(collector: FlowCollector<R>): Nothing {
        source.map(project).distinctUntilChanged().collect(collector)
        error("StateFlow collection completed")
    }
}
