package org.bemu.droid

import android.content.res.Configuration
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.foundation.clickable
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.min
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView

/** What the screen's controls do: the activity's. */
interface Controls {
    val handle: Long
    fun open()
    fun reset()
    fun settings()
    fun rulezCharge()
}

/**
 * The emulator's screen, laid out for the phone's orientation: in portrait the picture on top and
 * the joystick below it; in landscape the picture in the middle, the stick on its left and the fire
 * buttons on its right. The CPC keyboard, when asked for, is drawn over the picture, translucent.
 */
@Composable
fun BemuScreen(controls: Controls) {
    val landscape = LocalConfiguration.current.orientation == Configuration.ORIENTATION_LANDSCAPE
    var keyboard by remember { mutableStateOf(false) }
    // Back closes the keyboard before it leaves the app
    BackHandler(enabled = keyboard) {
        keyboard = false
        Native.releaseKeys(controls.handle)
    }
    val joystick = remember { JoystickState { bits -> Native.joystick(controls.handle, bits) } }
    val onKey = { number: Int, down: Boolean -> Native.key(controls.handle, number, down) }

    val picture = @Composable { modifier: Modifier ->
        Box(modifier) {
            EmulatorSurface(controls.handle, Modifier.fillMaxSize())
            if (keyboard) {
                CpcKeyboardOverlay(onKey = onKey, modifier = Modifier.fillMaxSize())
            }
        }
    }
    val toolbar = @Composable { modifier: Modifier, vertical: Boolean ->
        Toolbar(
            keyboard = keyboard,
            onKeyboard = {
                keyboard = !keyboard
                if (!keyboard) Native.releaseKeys(controls.handle)
            },
            controls = controls,
            vertical = vertical,
            modifier = modifier,
        )
    }

    Box(
        Modifier
            .fillMaxSize()
            .background(Color.Black)
            .safeDrawingPadding(),
    ) {
        if (landscape) {
            BoxWithConstraints(Modifier.fillMaxSize()) {
                // the picture as large as the height allows (4:3), the controls on its sides
                val side = ((maxWidth - maxHeight * 4f / 3f) / 2).coerceAtLeast(120.dp)
                Row(Modifier.fillMaxSize(), verticalAlignment = Alignment.CenterVertically) {
                    Column(
                        Modifier.width(side).fillMaxHeight().padding(8.dp),
                        verticalArrangement = Arrangement.SpaceBetween,
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        toolbar(Modifier, false)
                        Stick(
                            onDirections = joystick::directions,
                            modifier = Modifier.widthIn(max = min(side - 16.dp, 220.dp)).fillMaxWidth(),
                        )
                    }
                    picture(Modifier.weight(1f).fillMaxHeight())
                    Column(
                        Modifier.width(side).fillMaxHeight().padding(8.dp),
                        verticalArrangement = Arrangement.Bottom,
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        FireButtons(joystick, min(side - 16.dp, 200.dp))
                    }
                }
            }
        } else {
            Column(Modifier.fillMaxSize()) {
                toolbar(Modifier.fillMaxWidth().padding(4.dp), false)
                picture(Modifier.fillMaxWidth().aspectRatio(4f / 3f))
                // the joystick at the bottom
                Row(
                    Modifier.fillMaxWidth().weight(1f).padding(16.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    BoxWithConstraints(Modifier.weight(1f).fillMaxHeight(), contentAlignment = Alignment.CenterStart) {
                        Stick(
                            onDirections = joystick::directions,
                            modifier = Modifier.width(min(min(maxWidth, maxHeight), 240.dp)),
                        )
                    }
                    Spacer(Modifier.width(16.dp))
                    BoxWithConstraints(Modifier.weight(0.8f).fillMaxHeight(), contentAlignment = Alignment.Center) {
                        FireButtons(joystick, min(min(maxWidth, maxHeight * 1.4f), 200.dp))
                    }
                }
            }
        }
    }
}

/** Fire 1 and 2, the second above and right of the first (as a thumb reaches them). */
@Composable
private fun FireButtons(joystick: JoystickState, size: Dp) {
    val button = size * 0.48f
    Box(Modifier.width(size).height(size)) {
        FireButton(
            "1",
            onHeld = { joystick.fire(Joystick.FIRE1, it) },
            modifier = Modifier.width(button).align(Alignment.BottomStart),
        )
        FireButton(
            "2",
            onHeld = { joystick.fire(Joystick.FIRE2, it) },
            modifier = Modifier.width(button).align(Alignment.TopEnd),
        )
    }
}

@Composable
private fun Toolbar(
    keyboard: Boolean,
    onKeyboard: () -> Unit,
    controls: Controls,
    vertical: Boolean,
    modifier: Modifier,
) {
    val buttons = @Composable {
        ToolButton("Open", onClick = controls::open)
        ToolButton("Keyboard", selected = keyboard, onClick = onKeyboard)
        ToolButton("|RZ", onClick = controls::rulezCharge)
        ToolButton("Reset", onClick = controls::reset)
        ToolButton("⚙", onClick = controls::settings)
    }
    if (vertical) {
        Column(modifier, verticalArrangement = Arrangement.spacedBy(6.dp)) { buttons() }
    } else {
        // in landscape the side column is narrow: two lines of buttons
        androidx.compose.foundation.layout.FlowRow(
            modifier,
            horizontalArrangement = Arrangement.spacedBy(6.dp, Alignment.CenterHorizontally),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) { buttons() }
    }
}

@Composable
private fun ToolButton(label: String, selected: Boolean = false, onClick: () -> Unit) {
    Box(
        Modifier
            .background(if (selected) Color(0xCCFFD24D) else Color(0x44FFFFFF), RoundedCornerShape(6.dp))
            .clickable(onClick = onClick)
            .padding(horizontal = 10.dp, vertical = 6.dp),
    ) {
        Text(
            label,
            color = if (selected) Color.Black else Color.White,
            fontSize = 13.sp,
            fontWeight = FontWeight.Medium,
        )
    }
}

/** The surface the emulator draws in (from its own thread, in Rust). */
@Composable
private fun EmulatorSurface(handle: Long, modifier: Modifier) {
    AndroidView(
        modifier = modifier,
        factory = { context ->
            SurfaceView(context).apply {
                holder.addCallback(object : SurfaceHolder.Callback {
                    override fun surfaceCreated(holder: SurfaceHolder) {
                        Native.setSurface(handle, holder.surface)
                    }

                    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
                        Native.resize(handle, width, height)
                    }

                    override fun surfaceDestroyed(holder: SurfaceHolder) {
                        Native.setSurface(handle, null)
                    }
                })
            }
        },
    )
}
