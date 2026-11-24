package org.bemu.droid

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** The settings; applying them switches the machine off and on with them. */
@Composable
fun SettingsDialog(initial: Settings, onApply: (Settings) -> Unit, onDismiss: () -> Unit) {
    var s by remember { mutableStateOf(initial) }
    AlertDialog(
        onDismissRequest = onDismiss,
        confirmButton = { TextButton(onClick = { onApply(s) }) { Text("Apply (resets the CPC)") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
        title = { Text("Settings") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                Text("Model", fontSize = 14.sp)
                androidx.compose.foundation.layout.FlowRow {
                    Settings.MODELS.forEach { model ->
                        FilterChip(
                            selected = s.model == model,
                            onClick = { s = s.copy(model = model) },
                            label = { Text(model) },
                            modifier = Modifier.padding(end = 4.dp),
                        )
                    }
                }
                Toggle("CTM644 monitor (off: raw picture, if too slow)", s.ctm) { s = s.copy(ctm = it) }
                Toggle("Autorun discs and tapes", s.autorun) { s = s.copy(autorun = it) }
                Toggle("Sound", s.audio) { s = s.copy(audio = it) }
                Toggle("M4 board (SD card: the app's m4sd folder)", s.m4) { s = s.copy(m4 = it) }
                Toggle("M4 WiFi", s.m4Wifi) { s = s.copy(m4Wifi = it) }
                Toggle("RulezCharge (|RZ: downloads from cpcrulez.fr)", s.rulezcharge) { s = s.copy(rulezcharge = it) }
                Toggle("UniDOS", s.unidos) { s = s.copy(unidos = it) }
                Toggle("PlayCity", s.playcity) { s = s.copy(playcity = it) }
                Toggle("French ROMs and AZERTY (6128)", s.language == "fr") {
                    s = s.copy(language = if (it) "fr" else "en")
                }
            }
        },
    )
}

@Composable
private fun Toggle(label: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(vertical = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, Modifier.weight(1f), fontSize = 14.sp)
        Switch(checked = checked, onCheckedChange = onChange)
    }
}
