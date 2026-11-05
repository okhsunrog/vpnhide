package dev.okhsunrog.vpnhide.settings

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.res.stringResource
import dev.okhsunrog.vpnhide.CanonicalConfigRepository
import dev.okhsunrog.vpnhide.CanonicalMutation
import dev.okhsunrog.vpnhide.CoreSessions
import dev.okhsunrog.vpnhide.LocalConfigWriteAccess
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.core.CoreRootEffect
import kotlinx.coroutines.launch
import uniffi.vpnhide_core_ffi.CoreBackendChoiceRequest

/** Core supplies eligibility and upgrade deduplication; this only presents it. */
@Composable
internal fun BackendMigrationOffer() {
    val core by CoreSessions.updates.collectAsState()
    val status = core.state?.backendChoice ?: return
    val offer = status.migrationOffer ?: return
    if (!core.current || offer.kept) return
    var visible by remember(offer.sourceCommit) { mutableStateOf(!offer.displayed) }
    val scope = rememberCoroutineScope()
    val checkWrite = LocalConfigWriteAccess.current
    if (!visible) return
    LaunchedEffect(offer.sourceCommit) {
        submitBackendOfferEffect(CoreRootEffect.AcknowledgeBackendOffer(offer.sourceCommit, false))
    }
    val keep = {
        visible = false
        scope.launch { submitBackendOfferEffect(CoreRootEffect.AcknowledgeBackendOffer(offer.sourceCommit, true)) }
        Unit
    }
    AlertDialog(
        onDismissRequest = keep,
        title = { Text(stringResource(R.string.backend_migration_offer_title)) },
        text = { Text(stringResource(R.string.backend_migration_offer_body)) },
        confirmButton = {
            TextButton(onClick = {
                if (checkWrite()) {
                    visible = false
                    scope.launch {
                        submitBackendOfferEffect(
                            CoreRootEffect.StageBackendChoice(CoreBackendChoiceRequest(status.revision, offer.target, null)),
                        )
                    }
                }
            }) { Text(stringResource(R.string.backend_migration_offer_switch)) }
        },
        dismissButton = { TextButton(onClick = keep) { Text(stringResource(R.string.backend_migration_offer_keep)) } },
    )
}

private suspend fun submitBackendOfferEffect(effect: CoreRootEffect) {
    CanonicalConfigRepository.commit(
        CanonicalMutation(emptyList(), coupledEffects = listOf(effect), protectAllDrafts = true),
    )
}
