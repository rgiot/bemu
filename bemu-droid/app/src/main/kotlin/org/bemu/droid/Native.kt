package org.bemu.droid

import android.content.Context
import android.view.Surface

/**
 * The emulator, in Rust (libbemu_droid.so, see rust/src/jni_api.rs). Everything runs there: the
 * machine, the CRT drawn in the surface, the sound, the media and their autorun. A handle is one
 * emulator, its own thread.
 */
object Native {
    init {
        System.loadLibrary("bemu_droid")
    }

    /**
     * An emulator configured by [settings] (JSON, see [Settings]), its M4 SD card in [sdFolder]; 0 on
     * failure. The machine saved in [stateFolder] ([save]) comes back if it had these settings.
     */
    external fun create(context: Context, settings: String, sdFolder: String, stateFolder: String): Long

    /** The machine saved (settings, medium, snapshot), to come back if Android ends the app: null, or why not. */
    external fun save(handle: Long): String?

    external fun destroy(handle: Long)

    /** Drawn in [surface]; null when it is destroyed (returns once nothing draws in it). */
    external fun setSurface(handle: Long, surface: Surface?)

    external fun resize(handle: Long, width: Int, height: Int)

    external fun pause(handle: Long, paused: Boolean)

    /** A disc, tape, cartridge or snapshot: "ok:" and what autorun types, or "error:" and why. */
    external fun load(handle: Long, name: String, bytes: ByteArray): String

    external fun reset(handle: Long)

    /** New settings (JSON): null, or why they were refused. */
    external fun configure(handle: Long, settings: String): String?

    external fun defaultSettings(): String

    /** Typed on the CPC keyboard; CSL escapes: \(RET), \(ESC)... */
    external fun typeText(handle: Long, text: String)

    /** The CPC key of the firmware's [number] (matrix line * 8 + bit) held or released. */
    external fun key(handle: Long, number: Int, down: Boolean)

    external fun releaseKeys(handle: Long)

    /** [Joystick] bits: up 1, down 2, left 4, right 8, fire 1 16, fire 2 32. */
    external fun joystick(handle: Long, bits: Int)

    /**
     * The keycaps in [language] ("en", "fr"): 80 lines by the firmware's key numbers, "normal<TAB>shifted",
     * empty for a key that types no printable character.
     */
    external fun keyLegends(language: String): String

    /** Whether the phone was found too slow for the CRT since the last call (the picture is raw now). */
    external fun takeCrtDropped(handle: Long): Boolean

    /** The CRT back, too slow as it is (the user asked). */
    external fun keepCrt(handle: Long)

    /** Whether the GPU draws the CRT (else the picture is raw). */
    external fun hasCrt(handle: Long): Boolean

    /** A periodically updated graphics and timing snapshot. */
    external fun diagnostics(handle: Long): String
}
