package org.bemu.droid

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** A key: its label, the firmware's key number (matrix line * 8 + bit), its width in key units. */
data class CpcKey(val label: String, val number: Int, val width: Float = 1f)

/** The firmware's numbers of the keys the keyboard treats apart. */
object Keys {
    const val SHIFT = 21
    const val CONTROL = 23
}

private val MAIN_ROWS = listOf(
    listOf(
        CpcKey("ESC", 66), CpcKey("1", 64), CpcKey("2", 65), CpcKey("3", 57), CpcKey("4", 56),
        CpcKey("5", 49), CpcKey("6", 48), CpcKey("7", 41), CpcKey("8", 40), CpcKey("9", 33),
        CpcKey("0", 32), CpcKey("-", 25), CpcKey("^", 24), CpcKey("CLR", 16), CpcKey("DEL", 79),
    ),
    listOf(
        CpcKey("TAB", 68, 1.4f), CpcKey("Q", 67), CpcKey("W", 59), CpcKey("E", 58), CpcKey("R", 50),
        CpcKey("T", 51), CpcKey("Y", 43), CpcKey("U", 42), CpcKey("I", 35), CpcKey("O", 34),
        CpcKey("P", 27), CpcKey("@", 26), CpcKey("[", 17), CpcKey("RET", 18, 1.6f),
    ),
    listOf(
        CpcKey("CAPS", 70, 1.7f), CpcKey("A", 69), CpcKey("S", 60), CpcKey("D", 61), CpcKey("F", 53),
        CpcKey("G", 52), CpcKey("H", 44), CpcKey("J", 45), CpcKey("K", 37), CpcKey("L", 36),
        CpcKey(":", 29), CpcKey(";", 28), CpcKey("]", 19), CpcKey("RET", 18, 1.3f),
    ),
    listOf(
        CpcKey("SHIFT", Keys.SHIFT, 2.2f), CpcKey("Z", 71), CpcKey("X", 63), CpcKey("C", 62),
        CpcKey("V", 55), CpcKey("B", 54), CpcKey("N", 46), CpcKey("M", 38), CpcKey(",", 39),
        CpcKey(".", 31), CpcKey("/", 30), CpcKey("\\", 22), CpcKey("SHIFT", Keys.SHIFT, 1.8f),
    ),
    listOf(
        CpcKey("CTRL", Keys.CONTROL, 1.6f), CpcKey("COPY", 9, 1.6f), CpcKey("SPACE", 47, 6.2f),
        CpcKey("←", 8), CpcKey("↑", 0), CpcKey("↓", 2), CpcKey("→", 1), CpcKey("ENTER", 6, 1.6f),
    ),
)

/** The function keys (the numeric keypad) and the cursor keys, the second page. */
private val FUNCTION_ROWS = listOf(
    listOf(CpcKey("f7", 10), CpcKey("f8", 11), CpcKey("f9", 3), CpcKey("ESC", 66), CpcKey("DEL", 79), CpcKey("CLR", 16)),
    listOf(CpcKey("f4", 20), CpcKey("f5", 12), CpcKey("f6", 4), CpcKey("COPY", 9), CpcKey("↑", 0), CpcKey("RET", 18)),
    listOf(CpcKey("f1", 13), CpcKey("f2", 14), CpcKey("f3", 5), CpcKey("←", 8), CpcKey("↓", 2), CpcKey("→", 1)),
    listOf(CpcKey("f0", 15), CpcKey("f.", 7), CpcKey("ENTER", 6), CpcKey("SHIFT", Keys.SHIFT), CpcKey("CTRL", Keys.CONTROL), CpcKey("SPACE", 47)),
)

/**
 * The CPC's keyboard drawn over the picture, translucent: a phone's screen has no room for both.
 * Several keys can be held at once (multitouch). SHIFT and CTRL latch: touched, they stay down
 * until the next key is released.
 */
@Composable
fun CpcKeyboard(onKey: (number: Int, down: Boolean) -> Unit, modifier: Modifier = Modifier) {
    var functions by remember { mutableStateOf(false) }
    val latched = remember { mutableStateOf(setOf<Int>()) }
    val rows = if (functions) FUNCTION_ROWS else MAIN_ROWS

    fun press(key: CpcKey, down: Boolean) {
        val modifiers = latched.value
        when {
            key.number == Keys.SHIFT || key.number == Keys.CONTROL -> {
                if (down) {
                    val on = key.number !in modifiers
                    latched.value = if (on) modifiers + key.number else modifiers - key.number
                    onKey(key.number, on)
                }
            }
            down -> onKey(key.number, true)
            else -> {
                onKey(key.number, false)
                // a latched modifier applies to one key
                modifiers.forEach { onKey(it, false) }
                latched.value = emptySet()
            }
        }
    }

    Column(
        modifier = modifier
            .background(Color(0x66000000), RoundedCornerShape(8.dp))
            .padding(3.dp),
        verticalArrangement = Arrangement.spacedBy(3.dp),
    ) {
        rows.forEach { row ->
            Row(
                modifier = Modifier.fillMaxWidth().weight(1f),
                horizontalArrangement = Arrangement.spacedBy(3.dp),
            ) {
                row.forEach { key ->
                    KeyCap(
                        key = key,
                        latched = key.number in latched.value,
                        onPress = { down -> press(key, down) },
                        modifier = Modifier.weight(key.width).fillMaxHeight(),
                    )
                }
                if (row === rows.last()) {
                    KeyCap(
                        key = CpcKey(if (functions) "ABC" else "Fn", -1, 1.2f),
                        latched = false,
                        onPress = { down -> if (down) functions = !functions },
                        modifier = Modifier.weight(1.2f).fillMaxHeight(),
                    )
                }
            }
        }
    }
}

/**
 * What a keycap shows: the firmware's legend for a character key (the AZERTY one with the French
 * ROMs), the label of the table for the others (RETURN, the cursor keys...). A letter shows its
 * capital; another key its shifted character above the normal one, as on the CPC.
 */
fun keycapText(key: CpcKey, legends: Map<Int, Pair<String, String>>): String {
    if (key.label.length > 1) return key.label
    val (normal, shifted) = legends[key.number] ?: return key.label
    return when {
        // a letter key: Shift gives its capital (é is a letter, but its key's Shift gives 2)
        normal.length == 1 && normal[0].isLetter() && shifted == normal.uppercase() -> shifted
        normal == shifted -> normal
        else -> "$shifted\n$normal"
    }
}

@Composable
private fun KeyCap(key: CpcKey, latched: Boolean, onPress: (Boolean) -> Unit, modifier: Modifier) {
    var held by remember { mutableStateOf(false) }
    val feedback = touchFeedback()
    val text = keycapText(key, LocalKeyLegends.current)
    val background = when {
        held || latched -> Color(0xCCFFD24D)
        key.label.length > 1 -> Color(0x88505050)
        else -> Color(0x88303030)
    }
    Box(
        modifier = modifier
            .background(background, RoundedCornerShape(5.dp))
            .border(1.dp, Color(0x55FFFFFF), RoundedCornerShape(5.dp))
            .pointerInput(key) {
                detectTapGestures(onPress = {
                    held = true
                    feedback()
                    onPress(true)
                    tryAwaitRelease()
                    held = false
                    onPress(false)
                })
            },
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = text,
            color = if (held || latched) Color.Black else Color(0xEEFFFFFF),
            fontSize = when {
                text.length > 2 -> 10.sp
                '\n' in text -> 11.sp
                else -> 14.sp
            },
            lineHeight = 11.sp,
            fontWeight = FontWeight.Bold,
            textAlign = TextAlign.Center,
            maxLines = 2,
        )
    }
}

/** The keyboard over the box given (the picture), in its lower part. */
@Composable
fun CpcKeyboardOverlay(onKey: (Int, Boolean) -> Unit, modifier: Modifier = Modifier) {
    Box(modifier = modifier.fillMaxSize(), contentAlignment = Alignment.BottomCenter) {
        CpcKeyboard(onKey = onKey, modifier = Modifier.fillMaxWidth().fillMaxHeight(0.62f))
    }
}
