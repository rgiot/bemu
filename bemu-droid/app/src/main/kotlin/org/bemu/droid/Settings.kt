package org.bemu.droid

import android.content.Context
import android.content.res.Resources
import org.json.JSONObject
import java.util.Locale

/**
 * The app's settings. The emulator reads [toEngineJson] (rust/src/config.rs); the app keeps
 * [toPrefsJson], which also has what only the app uses. The defaults: a 6128 on its CTM644 colour
 * monitor, the M4 with WiFi and RulezCharge, UniDOS, autorun, and the language of the phone: the
 * French ROMs and the AZERTY keyboard on a French phone, the English ones otherwise. The CTM is the
 * most demanding: turn it off (raw picture) on a phone too slow for it.
 */
data class Settings(
    val model: String = "6128",
    /** "auto" (the phone's language), "en" or "fr". */
    val languageChoice: String = AUTO,
    val ctm: Boolean = true,
    val m4: Boolean = true,
    val m4Wifi: Boolean = true,
    val rulezcharge: Boolean = true,
    val unidos: Boolean = true,
    val playcity: Boolean = false,
    val autorun: Boolean = true,
    val audio: Boolean = true,
    /** A short vibration when a key, the stick or a fire button is touched. */
    val vibrate: Boolean = true,
) {
    /** The CPC's language: the one chosen, or the phone's ([locale]) with "auto". */
    fun language(locale: Locale = phoneLocale()): String =
        if (languageChoice == AUTO) languageFor(locale) else languageChoice

    /** What the emulator is configured with. */
    fun toEngineJson(locale: Locale = phoneLocale()): String = JSONObject()
        .put("model", model)
        .put("language", language(locale))
        .put("display", if (ctm) "ctm" else "raw")
        .put("m4", m4)
        .put("m4Wifi", m4Wifi)
        .put("rulezcharge", rulezcharge)
        .put("unidos", unidos)
        .put("playcity", playcity)
        .put("autorun", autorun)
        .put("audio", audio)
        .toString()

    /** What the app saves. */
    fun toPrefsJson(): String = JSONObject()
        .put("model", model)
        .put("languageChoice", languageChoice)
        .put("ctm", ctm)
        .put("m4", m4)
        .put("m4Wifi", m4Wifi)
        .put("rulezcharge", rulezcharge)
        .put("unidos", unidos)
        .put("playcity", playcity)
        .put("autorun", autorun)
        .put("audio", audio)
        .put("vibrate", vibrate)
        .toString()

    fun save(context: Context) {
        context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE).edit()
            .putString(KEY, toPrefsJson())
            .apply()
    }

    companion object {
        private const val PREFERENCES = "bemu"
        private const val KEY = "settings"
        const val AUTO = "auto"

        val MODELS = listOf("464", "664", "6128", "6128plus", "464plus", "gx4000")

        /** The CPC's language for a phone in [locale]: French for any French locale, English otherwise. */
        fun languageFor(locale: Locale): String = if (locale.language == "fr") "fr" else "en"

        /** The phone's language (the system's, not only this app's). */
        fun phoneLocale(): Locale = Resources.getSystem().configuration.locales[0] ?: Locale.getDefault()

        fun load(context: Context): Settings {
            val json = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE).getString(KEY, null)
            return json?.let(::fromPrefsJson) ?: Settings()
        }

        /** The saved settings; null when they cannot be read. */
        fun fromPrefsJson(json: String): Settings? = try {
            val o = JSONObject(json)
            val d = Settings()
            Settings(
                model = o.optString("model", d.model),
                // the first version saved "language" (English unless chosen): the phone's from now on
                languageChoice = o.optString("languageChoice", AUTO),
                ctm = if (o.has("ctm")) o.optBoolean("ctm") else o.optString("display", "ctm") != "raw",
                m4 = o.optBoolean("m4", d.m4),
                m4Wifi = o.optBoolean("m4Wifi", d.m4Wifi),
                rulezcharge = o.optBoolean("rulezcharge", d.rulezcharge),
                unidos = o.optBoolean("unidos", d.unidos),
                playcity = o.optBoolean("playcity", d.playcity),
                autorun = o.optBoolean("autorun", d.autorun),
                audio = o.optBoolean("audio", d.audio),
                vibrate = o.optBoolean("vibrate", d.vibrate),
            )
        } catch (e: Exception) {
            null
        }
    }
}
