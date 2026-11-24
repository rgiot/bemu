package org.bemu.droid

import android.content.Context
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import org.json.JSONObject

/** A component of the app that is not bemu's own (third_party/licenses.json, written by tools/licenses.py). */
data class Component(
    val kind: String,
    val name: String,
    val version: String,
    val license: String,
    val owner: String?,
    val url: String?,
    val note: String?,
    /** (title, text) of its licence texts. */
    val texts: List<Pair<String, String>>,
)

/** The components of the Android build, from the app's assets (copied there by the Gradle build). */
fun loadComponents(context: Context): List<Component> {
    val json = JSONObject(context.assets.open("licenses.json").bufferedReader().use { it.readText() })
    val texts = json.getJSONObject("texts")
    val components = json.getJSONArray("components")
    return (0 until components.length()).map { components.getJSONObject(it) }
        .filter { c -> (0 until c.getJSONArray("builds").length()).any { c.getJSONArray("builds").getString(it) == "android" } }
        .map { c ->
            val list = c.getJSONArray("texts")
            Component(
                kind = c.getString("kind"),
                name = c.getString("name"),
                version = c.optString("version", ""),
                license = c.getString("license"),
                owner = c.optString("owner").takeIf { c.has("owner") && !c.isNull("owner") },
                url = c.optString("url").takeIf { c.has("url") && !c.isNull("url") },
                note = c.optString("note").takeIf { c.has("note") && !c.isNull("note") },
                texts = (0 until list.length()).map { i ->
                    val t = list.getJSONObject(i)
                    t.getString("name") to texts.optString(t.getString("text"))
                },
            )
        }
}

private fun kindTitle(kind: String) = when (kind) {
    "rom" -> "ROMs"
    "c-library" -> "C and Z80 code"
    "crate" -> "Rust libraries"
    "android" -> "Android libraries"
    else -> "Other"
}

/**
 * Settings > Licences: the app's licence, then everything in it that is not bemu's own (the ROMs,
 * the libraries), each with its licence's text.
 */
@Composable
fun LicencesScreen(onClose: () -> Unit) {
    val context = LocalContext.current
    val components = remember { runCatching { loadComponents(context) }.getOrDefault(emptyList()) }
    val gpl = remember {
        runCatching { context.assets.open("LICENSE-GPL-3.0").bufferedReader().use { it.readText() } }.getOrDefault("")
    }
    var filter by remember { mutableStateOf("") }
    var open by remember { mutableStateOf(setOf<String>()) }

    Dialog(onDismissRequest = onClose, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Column(
            Modifier
                .fillMaxSize()
                .background(Color(0xFF15151C))
                .padding(12.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Licences", fontSize = 20.sp, fontWeight = FontWeight.Bold, color = Color.White, modifier = Modifier.weight(1f))
                TextButton(onClick = onClose) { Text("Close") }
            }
            Text(
                "bemu's source code is under the MIT or the Apache-2.0 licence. This app links the HxC " +
                    "floppy library (GPL-3.0), so the app itself is distributed under the GNU GPL, version 3. " +
                    "Its source: https://github.com/rgiot/bemu (and https://github.com/cpcsdk/rust.cpclib). " +
                    "Because of the ROMs' terms and part of HxC, it must be given away, not sold. " +
                    "What follows is in the app and is not bemu's own: each keeps its owners' licence.",
                color = Color(0xFFDDDDDD),
                fontSize = 13.sp,
            )
            OutlinedTextField(
                value = filter,
                onValueChange = { filter = it },
                label = { Text("Filter") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth().padding(vertical = 6.dp),
            )
            val f = filter.lowercase()
            val shown = components.filter {
                f.isEmpty() || it.name.lowercase().contains(f) || it.license.lowercase().contains(f) ||
                    (it.owner ?: "").lowercase().contains(f)
            }
            LazyColumn(Modifier.fillMaxSize()) {
                item {
                    Entry(
                        title = "This app — GNU General Public License, version 3",
                        expanded = "gpl" in open,
                        onToggle = { open = if ("gpl" in open) open - "gpl" else open + "gpl" },
                    ) { LicenceText(gpl) }
                }
                var kind = ""
                for (component in shown) {
                    if (component.kind != kind) {
                        kind = component.kind
                        item(key = "kind-$kind") {
                            Text(
                                kindTitle(component.kind),
                                color = Color(0xFFFFD24D),
                                fontWeight = FontWeight.Bold,
                                modifier = Modifier.padding(top = 10.dp, bottom = 4.dp),
                            )
                        }
                    }
                    val key = "${component.kind}/${component.name}/${component.version}"
                    item(key = key) {
                        val version = if (component.version.isEmpty()) "" else " ${component.version}"
                        Entry(
                            title = "${component.name}$version — ${component.license}",
                            expanded = key in open,
                            onToggle = { open = if (key in open) open - key else open + key },
                        ) {
                            component.owner?.let { Text("By $it", color = Color(0xFFBBBBBB), fontSize = 12.sp) }
                            component.url?.let { Text(it, color = Color(0xFF80B0FF), fontSize = 12.sp) }
                            component.note?.let { Text(it, color = Color(0xFFBBBBBB), fontSize = 12.sp) }
                            component.texts.forEach { (title, text) ->
                                Text(title, color = Color.White, fontWeight = FontWeight.Medium, fontSize = 12.sp, modifier = Modifier.padding(top = 6.dp))
                                LicenceText(text)
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun Entry(title: String, expanded: Boolean, onToggle: () -> Unit, content: @Composable () -> Unit) {
    Column(Modifier.fillMaxWidth()) {
        Text(
            (if (expanded) "▾ " else "▸ ") + title,
            color = Color.White,
            fontSize = 14.sp,
            modifier = Modifier.fillMaxWidth().clickable(onClick = onToggle).padding(vertical = 6.dp),
        )
        if (expanded) {
            Column(Modifier.padding(start = 16.dp, bottom = 8.dp)) { content() }
        }
        HorizontalDivider(color = Color(0x22FFFFFF))
    }
}

@Composable
private fun LicenceText(text: String) {
    Text(
        text,
        color = Color(0xFFCCCCCC),
        fontFamily = FontFamily.Monospace,
        fontSize = 10.sp,
        softWrap = true,
        modifier = Modifier.horizontalScroll(rememberScrollState()).padding(top = 2.dp),
    )
}
