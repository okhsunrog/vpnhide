package dev.okhsunrog.vpnhide.picker

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Clear
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.ui.components.container

/**
 * The in-place search/filter top bar shared by the app-picker and hidden-apps
 * screens: a full-width text field whose leading arrow closes search ([onClose])
 * and whose trailing clear button appears only when [query] is non-empty. It
 * narrows the list *behind* it live — this is a filter, not an M3 search view
 * (which is a search-and-navigate pattern with an expanded results surface), so
 * it's a plain [TextField], not the SearchBar/SearchBarState family (whose docked
 * bar disables the soft keyboard and expects a full-screen expansion).
 *
 * [query] stays owned by the caller (it also drives the filtering upstream).
 */
@Composable
internal fun AppSearchTopBar(
    query: String,
    onQueryChange: (String) -> Unit,
    onClose: () -> Unit,
    modifier: Modifier = Modifier,
) {
    // Search is entered by an explicit user action (tapping the search icon), so
    // focus the field and raise the keyboard as soon as the bar appears.
    val focusRequester = remember { FocusRequester() }
    LaunchedEffect(Unit) { focusRequester.requestFocus() }

    TextField(
        value = query,
        onValueChange = onQueryChange,
        modifier = modifier.statusBarsPadding().fillMaxWidth().focusRequester(focusRequester),
        singleLine = true,
        placeholder = { Text(stringResource(R.string.search_placeholder)) },
        leadingIcon = {
            IconButton(onClick = onClose) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = null)
            }
        },
        trailingIcon = {
            if (query.isNotEmpty()) {
                IconButton(onClick = { onQueryChange("") }) {
                    Icon(Icons.Default.Clear, contentDescription = null)
                }
            }
        },
        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
        // Blend into the top bar: no filled container, no indicator underline.
        colors =
            TextFieldDefaults.colors(
                focusedContainerColor = Color.Transparent,
                unfocusedContainerColor = Color.Transparent,
                focusedIndicatorColor = Color.Transparent,
                unfocusedIndicatorColor = Color.Transparent,
                disabledIndicatorColor = Color.Transparent,
            ),
    )
}
