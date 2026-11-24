package org.bemu.droid

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlin.math.atan2
import kotlin.math.hypot
import kotlin.math.min

/** The joystick's bits, as the emulator reads them ([Native.joystick]). */
object Joystick {
    const val UP = 1
    const val DOWN = 2
    const val LEFT = 4
    const val RIGHT = 8
    const val FIRE1 = 16
    const val FIRE2 = 32
    const val DIRECTIONS = UP or DOWN or LEFT or RIGHT
}

/** The bits of a stick pushed by ([dx], [dy]) of a [radius]: 8 directions, a dead zone in the middle. */
fun stickBits(dx: Float, dy: Float, radius: Float): Int {
    if (hypot(dx, dy) < radius * 0.25f) return 0
    // eight sectors of 45°, from the right, clockwise (y goes down)
    val sector = ((Math.toDegrees(atan2(dy, dx).toDouble()) + 360 + 22.5) % 360 / 45).toInt()
    return when (sector) {
        0 -> Joystick.RIGHT
        1 -> Joystick.RIGHT or Joystick.DOWN
        2 -> Joystick.DOWN
        3 -> Joystick.DOWN or Joystick.LEFT
        4 -> Joystick.LEFT
        5 -> Joystick.LEFT or Joystick.UP
        6 -> Joystick.UP
        else -> Joystick.UP or Joystick.RIGHT
    }
}

/** A stick: a thumb dragged from where it touches; [onDirections] gets the direction bits. */
@Composable
fun Stick(onDirections: (Int) -> Unit, modifier: Modifier = Modifier) {
    val feedback = touchFeedback()
    var knob by remember { mutableStateOf(Offset.Zero) }
    var bits by remember { mutableStateOf(0) }
    Canvas(
        modifier = modifier
            .aspectRatio(1f)
            .pointerInput(Unit) {
                awaitPointerEventScope {
                    while (true) {
                        val event = awaitPointerEvent()
                        val change = event.changes.firstOrNull { it.pressed }
                        val center = Offset(size.width / 2f, size.height / 2f)
                        val radius = min(size.width, size.height) / 2f
                        val newBits = if (change == null || event.type == PointerEventType.Release && event.changes.none { it.pressed }) {
                            knob = Offset.Zero
                            0
                        } else {
                            val d = change.position - center
                            val length = hypot(d.x, d.y)
                            knob = if (length > radius) d * (radius / length) else d
                            change.consume()
                            stickBits(d.x, d.y, radius)
                        }
                        if (newBits != bits) {
                            if (newBits != 0) feedback()
                            bits = newBits
                            onDirections(newBits)
                        }
                    }
                }
            },
    ) {
        val radius = min(size.width, size.height) / 2f
        drawCircle(Color(0x33FFFFFF), radius)
        drawCircle(Color(0x88FFFFFF), radius, style = Stroke(width = 3f))
        // the eight directions
        for (i in 0 until 8) {
            val angle = Math.toRadians(i * 45.0)
            val at = center + Offset(kotlin.math.cos(angle).toFloat(), kotlin.math.sin(angle).toFloat()) * (radius * 0.8f)
            drawCircle(Color(0x55FFFFFF), radius * 0.04f, at)
        }
        val lit = bits != 0
        drawCircle(if (lit) Color(0xDDFFD24D) else Color(0xAAAAAAAA), radius * 0.38f, center + knob * 0.62f)
    }
}

/** A fire button: held while touched. */
@Composable
fun FireButton(label: String, onHeld: (Boolean) -> Unit, modifier: Modifier = Modifier) {
    val feedback = touchFeedback()
    var held by remember { mutableStateOf(false) }
    Box(
        modifier = modifier
            .aspectRatio(1f)
            .background(if (held) Color(0xDDFF4040) else Color(0x66FF4040), CircleShape)
            .border(2.dp, Color(0x88FFFFFF), CircleShape)
            .pointerInput(Unit) {
                detectTapGestures(onPress = {
                    held = true
                    feedback()
                    onHeld(true)
                    tryAwaitRelease()
                    held = false
                    onHeld(false)
                })
            },
        contentAlignment = Alignment.Center,
    ) {
        Text(label, color = Color.White, fontSize = 18.sp, fontWeight = FontWeight.Bold)
    }
}

/** The joystick's state, the stick's and the buttons' bits put together for the emulator. */
class JoystickState(private val send: (Int) -> Unit) {
    private var directions = 0
    private var fire = 0

    fun directions(bits: Int) {
        directions = bits and Joystick.DIRECTIONS
        send(directions or fire)
    }

    fun fire(bit: Int, held: Boolean) {
        fire = if (held) fire or bit else fire and bit.inv()
        send(directions or fire)
    }
}
