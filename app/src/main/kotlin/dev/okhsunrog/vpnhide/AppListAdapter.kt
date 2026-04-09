package dev.okhsunrog.vpnhide

import android.graphics.drawable.Drawable
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.CheckBox
import android.widget.ImageView
import android.widget.TextView
import androidx.recyclerview.widget.RecyclerView

data class AppInfo(
    val label: String,
    val packageName: String,
    val icon: Drawable?,
    var isHidden: Boolean,
    val isUserApp: Boolean,
)

class AppListAdapter(
    private val items: List<AppInfo>,
    private val onToggle: (AppInfo, Boolean) -> Unit,
) : RecyclerView.Adapter<AppListAdapter.ViewHolder>() {

    class ViewHolder(view: View) : RecyclerView.ViewHolder(view) {
        val icon: ImageView = view.findViewById(R.id.app_icon)
        val label: TextView = view.findViewById(R.id.app_label)
        val packageName: TextView = view.findViewById(R.id.app_package)
        val checkbox: CheckBox = view.findViewById(R.id.app_checkbox)
    }

    override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): ViewHolder {
        val view = LayoutInflater.from(parent.context)
            .inflate(R.layout.item_app, parent, false)
        return ViewHolder(view)
    }

    override fun onBindViewHolder(holder: ViewHolder, position: Int) {
        val app = items[position]
        holder.icon.setImageDrawable(app.icon)
        holder.label.text = app.label
        holder.packageName.text = app.packageName

        holder.checkbox.setOnCheckedChangeListener(null)
        holder.checkbox.isChecked = app.isHidden
        holder.checkbox.setOnCheckedChangeListener { _, isChecked ->
            app.isHidden = isChecked
            onToggle(app, isChecked)
        }

        holder.itemView.setOnClickListener {
            holder.checkbox.toggle()
        }
    }

    override fun getItemCount() = items.size
}
