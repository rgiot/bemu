package org.bemu.droid

import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback

/** Whether the touch controls vibrate (Settings > Vibrate on touch). */
val LocalVibrate = compositionLocalOf { true }

/** The keycaps of the CPC's keyboard, by the firmware's key numbers: (normal, shifted). */
val LocalKeyLegends = compositionLocalOf { emptyMap<Int, Pair<String, String>>() }

/** A short vibration for a key, a fire button or a stick direction, unless turned off. */
@Composable
fun touchFeedback(): () -> Unit {
    val haptic = LocalHapticFeedback.current
    // read when touched: the gesture handlers keep the lambda they started with
    val vibrate by rememberUpdatedState(LocalVibrate.current)
    return remember(haptic) { { if (vibrate) haptic.performHapticFeedback(HapticFeedbackType.VirtualKey) } }
}

/** [Native.keyLegends]' text as a map. */
fun parseKeyLegends(text: String): Map<Int, Pair<String, String>> =
    text.split('\n').withIndex()
        .filter { it.value.contains('\t') }
        .associate { (index, line) -> index to (line.substringBefore('\t') to line.substringAfter('\t')) }
