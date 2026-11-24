package org.bemu.droid

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import android.view.InputDevice
import android.view.KeyEvent
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.core.content.IntentCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import java.io.File
import kotlin.concurrent.thread

/**
 * The app's one activity: the emulator (in Rust, see [Native]) on the screen, the media given to it
 * from the file picker or from other apps ("Open with", "Share"), started by themselves.
 */
class MainActivity : ComponentActivity(), Controls {
    override var handle = 0L
        private set
    private var settings by mutableStateOf(Settings())
    private var showSettings by mutableStateOf(false)
    private var showLicences by mutableStateOf(false)

    private val picker = registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        uri?.let(::load)
    }

    /** The M4's SD card: a folder of the app's on the shared storage, what RulezCharge downloads. */
    private val sdFolder: File
        get() = File(getExternalFilesDir(null) ?: filesDir, "m4sd").apply { mkdirs() }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        fullScreen()
        settings = Settings.load(this)
        Licences.preload(this)
        handle = Native.create(this, settings.toJson(), sdFolder.absolutePath)
        if (handle == 0L) {
            Toast.makeText(this, "The emulator could not start", Toast.LENGTH_LONG).show()
        }
        setContent {
            MaterialTheme(colorScheme = darkColorScheme()) {
                BemuScreen(this)
                if (showSettings) {
                    SettingsDialog(
                        initial = settings,
                        onApply = { applySettings(it) },
                        onLicences = {
                            showSettings = false
                            showLicences = true
                        },
                        onDismiss = { showSettings = false },
                    )
                }
                if (showLicences) {
                    LicencesScreen(onClose = { showLicences = false })
                }
            }
        }
        if (savedInstanceState == null) {
            handleIntent(intent)
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleIntent(intent)
    }

    override fun onResume() {
        super.onResume()
        fullScreen()
        Native.pause(handle, false)
    }

    override fun onPause() {
        Native.pause(handle, true)
        super.onPause()
    }

    override fun onDestroy() {
        if (handle != 0L) {
            Native.destroy(handle)
            handle = 0L
        }
        super.onDestroy()
    }

    private fun fullScreen() {
        WindowCompat.getInsetsController(window, window.decorView).apply {
            hide(WindowInsetsCompat.Type.systemBars())
            systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        }
    }

    private fun handleIntent(intent: Intent?) {
        val uri = when (intent?.action) {
            Intent.ACTION_VIEW -> intent.data
            Intent.ACTION_SEND -> IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)
            else -> null
        }
        uri?.let(::load)
    }

    /** The file at [uri] read and given to the emulator, off the UI thread. */
    private fun load(uri: Uri) {
        thread(name = "bemu-load") {
            val name = displayName(uri)
            val answer = try {
                val bytes = contentResolver.openInputStream(uri)?.use(::readAtMost)
                if (bytes == null) "error:$name: unreadable or too big" else Native.load(handle, name, bytes)
            } catch (e: Exception) {
                "error:$name: ${e.message}"
            }
            runOnUiThread {
                val message = when {
                    answer.startsWith("error:") -> answer.removePrefix("error:")
                    answer == "ok:" -> name
                    else -> "$name: ${plain(answer.removePrefix("ok:"))}"
                }
                Toast.makeText(this, message, Toast.LENGTH_SHORT).show()
            }
        }
    }

    /** The stream's bytes, none beyond [MAX_FILE]. */
    private fun readAtMost(input: java.io.InputStream): ByteArray? {
        val out = java.io.ByteArrayOutputStream()
        val buffer = ByteArray(64 * 1024)
        while (true) {
            val read = input.read(buffer)
            if (read < 0) return out.toByteArray()
            out.write(buffer, 0, read)
            if (out.size() > MAX_FILE) return null
        }
    }

    private fun displayName(uri: Uri): String {
        if (uri.scheme == "content") {
            contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
                if (cursor.moveToFirst()) {
                    cursor.getString(0)?.let { return it }
                }
            }
        }
        return uri.lastPathSegment?.substringAfterLast('/') ?: "file"
    }

    private fun applySettings(new: Settings) {
        showSettings = false
        val error = Native.configure(handle, new.toJson())
        if (error != null) {
            Toast.makeText(this, error, Toast.LENGTH_LONG).show()
        } else {
            settings = new
            new.save(this)
        }
    }

    // Controls

    override fun open() {
        picker.launch(arrayOf("*/*"))
    }

    override fun reset() {
        Native.reset(handle)
    }

    override fun settings() {
        showSettings = true
    }

    override fun rulezCharge() {
        if (!settings.m4 || !settings.rulezcharge) {
            Toast.makeText(this, "RulezCharge needs the M4 and RulezCharge (settings)", Toast.LENGTH_LONG).show()
        }
        Native.typeText(handle, "|RZ\\(RET)")
    }

    // a hardware keyboard and a gamepad

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean =
        physicalKey(keyCode, event, true) || super.onKeyDown(keyCode, event)

    override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean =
        physicalKey(keyCode, event, false) || super.onKeyUp(keyCode, event)

    private var padBits = 0

    private fun physicalKey(keyCode: Int, event: KeyEvent, down: Boolean): Boolean {
        if (event.repeatCount > 0) return true
        val gamepad = event.source and (InputDevice.SOURCE_GAMEPAD or InputDevice.SOURCE_JOYSTICK) != 0
        if (gamepad) {
            val bit = when (keyCode) {
                KeyEvent.KEYCODE_DPAD_UP -> Joystick.UP
                KeyEvent.KEYCODE_DPAD_DOWN -> Joystick.DOWN
                KeyEvent.KEYCODE_DPAD_LEFT -> Joystick.LEFT
                KeyEvent.KEYCODE_DPAD_RIGHT -> Joystick.RIGHT
                KeyEvent.KEYCODE_BUTTON_A -> Joystick.FIRE1
                KeyEvent.KEYCODE_BUTTON_B -> Joystick.FIRE2
                else -> 0
            }
            if (bit != 0) {
                padBits = if (down) padBits or bit else padBits and bit.inv()
                Native.joystick(handle, padBits)
                return true
            }
        }
        val number = KEYS[keyCode] ?: return false
        Native.key(handle, number, down)
        return true
    }

    companion object {
        /** Media bigger than this are refused (a DSK is a few hundred K). */
        private const val MAX_FILE = 32 * 1024 * 1024

        /** The autorun's text in plain words: RUN"DISC rather than RUN"DISC\(RET). */
        fun plain(typed: String) = typed.replace(Regex("""\\\((RET|WAIT [^)]*)\)"""), " ").trim()

        /** A hardware keyboard's keys, to the CPC's (the firmware's numbers). */
        private val KEYS: Map<Int, Int> = buildMap {
            val letters = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
            val letterNumbers = intArrayOf(
                69, 54, 62, 61, 58, 53, 52, 44, 35, 45, 37, 36, 38, 46, 34, 27, 67, 50, 60, 51, 42, 55, 59, 63, 43, 71,
            )
            letters.forEachIndexed { i, _ -> put(KeyEvent.KEYCODE_A + i, letterNumbers[i]) }
            val digitNumbers = intArrayOf(32, 64, 65, 57, 56, 49, 48, 41, 40, 33)
            for (d in 0..9) put(KeyEvent.KEYCODE_0 + d, digitNumbers[d])
            put(KeyEvent.KEYCODE_SPACE, 47)
            put(KeyEvent.KEYCODE_ENTER, 18)
            put(KeyEvent.KEYCODE_NUMPAD_ENTER, 6)
            put(KeyEvent.KEYCODE_DEL, 79)
            put(KeyEvent.KEYCODE_FORWARD_DEL, 16)
            put(KeyEvent.KEYCODE_ESCAPE, 66)
            put(KeyEvent.KEYCODE_TAB, 68)
            put(KeyEvent.KEYCODE_SHIFT_LEFT, 21)
            put(KeyEvent.KEYCODE_SHIFT_RIGHT, 21)
            put(KeyEvent.KEYCODE_CTRL_LEFT, 23)
            put(KeyEvent.KEYCODE_CTRL_RIGHT, 23)
            put(KeyEvent.KEYCODE_ALT_LEFT, 9)
            put(KeyEvent.KEYCODE_CAPS_LOCK, 70)
            put(KeyEvent.KEYCODE_DPAD_UP, 0)
            put(KeyEvent.KEYCODE_DPAD_RIGHT, 1)
            put(KeyEvent.KEYCODE_DPAD_DOWN, 2)
            put(KeyEvent.KEYCODE_DPAD_LEFT, 8)
            put(KeyEvent.KEYCODE_MINUS, 25)
            put(KeyEvent.KEYCODE_EQUALS, 24)
            put(KeyEvent.KEYCODE_LEFT_BRACKET, 26)
            put(KeyEvent.KEYCODE_RIGHT_BRACKET, 17)
            put(KeyEvent.KEYCODE_BACKSLASH, 19)
            put(KeyEvent.KEYCODE_SEMICOLON, 29)
            put(KeyEvent.KEYCODE_APOSTROPHE, 28)
            put(KeyEvent.KEYCODE_COMMA, 39)
            put(KeyEvent.KEYCODE_PERIOD, 31)
            put(KeyEvent.KEYCODE_SLASH, 30)
            put(KeyEvent.KEYCODE_GRAVE, 22)
            for (f in 0..9) {
                put(KeyEvent.KEYCODE_NUMPAD_0 + f, intArrayOf(15, 13, 14, 5, 20, 12, 4, 10, 11, 3)[f])
            }
            put(KeyEvent.KEYCODE_NUMPAD_DOT, 7)
        }
    }
}
