package dev.okhsunrog.vpnhide

import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.os.Bundle
import android.view.Menu
import android.view.MenuItem
import androidx.appcompat.app.AppCompatActivity
import androidx.lifecycle.lifecycleScope
import androidx.recyclerview.widget.LinearLayoutManager
import androidx.recyclerview.widget.RecyclerView
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class MainActivity : AppCompatActivity() {

    private lateinit var recyclerView: RecyclerView
    private var allApps: List<AppInfo> = emptyList()
    private var currentFilter: Filter = Filter.USER

    private enum class Filter { USER, ALL }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)
        supportActionBar?.title = getString(R.string.app_name)

        recyclerView = findViewById(R.id.recycler_view)
        recyclerView.layoutManager = LinearLayoutManager(this)

        loadApps()
    }

    override fun onCreateOptionsMenu(menu: Menu): Boolean {
        menu.add(0, MENU_TOGGLE_FILTER, 0, R.string.show_system_apps)
            .setShowAsAction(MenuItem.SHOW_AS_ACTION_NEVER)
        return true
    }

    override fun onPrepareOptionsMenu(menu: Menu): Boolean {
        menu.findItem(MENU_TOGGLE_FILTER)?.setTitle(
            if (currentFilter == Filter.USER) R.string.show_system_apps
            else R.string.show_user_apps_only
        )
        return super.onPrepareOptionsMenu(menu)
    }

    override fun onOptionsItemSelected(item: MenuItem): Boolean {
        if (item.itemId == MENU_TOGGLE_FILTER) {
            currentFilter = if (currentFilter == Filter.USER) Filter.ALL else Filter.USER
            invalidateOptionsMenu()
            applyFilter()
            return true
        }
        return super.onOptionsItemSelected(item)
    }

    private fun loadApps() {
        lifecycleScope.launch {
            val apps = withContext(Dispatchers.IO) { queryInstalledApps() }
            allApps = apps
            applyFilter()
        }
    }

    private fun applyFilter() {
        val visible = when (currentFilter) {
            Filter.USER -> allApps.filter { it.isUserApp }
            Filter.ALL -> allApps
        }
        recyclerView.adapter = AppListAdapter(visible) { _, _ ->
            savePrefs(allApps)
        }
    }

    private fun queryInstalledApps(): List<AppInfo> {
        val pm = packageManager
        val prefs = getSharedPreferences(PrefsHelper.PREFS_NAME, MODE_PRIVATE)
        val hiddenPackages =
            prefs.getStringSet(PrefsHelper.KEY_HIDDEN_PACKAGES, emptySet()) ?: emptySet()

        return pm.getInstalledApplications(PackageManager.GET_META_DATA)
            .filter { it.packageName != packageName }
            .map { info ->
                AppInfo(
                    label = pm.getApplicationLabel(info).toString(),
                    packageName = info.packageName,
                    icon = runCatching { pm.getApplicationIcon(info) }.getOrNull(),
                    isHidden = info.packageName in hiddenPackages,
                    isUserApp = (info.flags and ApplicationInfo.FLAG_SYSTEM) == 0,
                )
            }
            .sortedWith(
                compareByDescending<AppInfo> { it.isUserApp }
                    .thenBy { it.label.lowercase() }
            )
    }

    private fun savePrefs(apps: List<AppInfo>) {
        val hidden = apps.filter { it.isHidden }.map { it.packageName }.toSet()
        getSharedPreferences(PrefsHelper.PREFS_NAME, MODE_PRIVATE)
            .edit()
            .putStringSet(PrefsHelper.KEY_HIDDEN_PACKAGES, hidden)
            .apply()
        // Bump the modification time so XSharedPreferences picks up the change
        // on next reload() inside hooked processes.
    }

    companion object {
        private const val MENU_TOGGLE_FILTER = 1
    }
}
