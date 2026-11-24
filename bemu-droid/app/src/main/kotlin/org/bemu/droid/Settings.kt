package org.bemu.droid

import android.content.Context
import org.json.JSONObject

/**
 * The app's settings, as the emulator reads them (rust/src/config.rs). The defaults: a 6128 on its
 * CTM644 colour monitor, the M4 with WiFi and RulezCharge, UniDOS, autorun. The CTM is the most
 * demanding: turn it off (raw picture) on a phone too slow for it.
 */
data class Settings(
    val model: String = "6128",
    val language: String = "en",
    val ctm: Boolean = true,
    val m4: Boolean = true,
    val m4Wifi: Boolean = true,
    val rulezcharge: Boolean = true,
    val unidos: Boolean = true,
    val playcity: Boolean = false,
    val autorun: Boolean = true,
    val audio: Boolean = true,
) {
    fun toJson(): String = JSONObject()
        .put("model", model)
        .put("language", language)
        .put("display", if (ctm) "ctm" else "raw")
        .put("m4", m4)
        .put("m4Wifi", m4Wifi)
        .put("rulezcharge", rulezcharge)
        .put("unidos", unidos)
        .put("playcity", playcity)
        .put("autorun", autorun)
        .put("audio", audio)
        .toString()

    fun save(context: Context) {
        context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE).edit()
            .putString(KEY, toJson())
            .apply()
    }

    companion object {
        private const val PREFERENCES = "bemu"
        private const val KEY = "settings"

        val MODELS = listOf("464", "664", "6128", "6128plus", "464plus", "gx4000")

        fun load(context: Context): Settings {
            val json = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE).getString(KEY, null)
                ?: return Settings()
            return try {
                val o = JSONObject(json)
                val d = Settings()
                Settings(
                    model = o.optString("model", d.model),
                    language = o.optString("language", d.language),
                    ctm = o.optString("display", "ctm") != "raw",
                    m4 = o.optBoolean("m4", d.m4),
                    m4Wifi = o.optBoolean("m4Wifi", d.m4Wifi),
                    rulezcharge = o.optBoolean("rulezcharge", d.rulezcharge),
                    unidos = o.optBoolean("unidos", d.unidos),
                    playcity = o.optBoolean("playcity", d.playcity),
                    autorun = o.optBoolean("autorun", d.autorun),
                    audio = o.optBoolean("audio", d.audio),
                )
            } catch (e: Exception) {
                Settings()
            }
        }
    }
}
