package org.bemu.droid

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Test
import java.util.Locale

class StickTest {
    @Test
    fun theStickHasEightDirectionsAndADeadZone() {
        val r = 100f
        assertEquals(0, stickBits(10f, 10f, r))
        assertEquals(Joystick.RIGHT, stickBits(80f, 0f, r))
        assertEquals(Joystick.LEFT, stickBits(-80f, 5f, r))
        // y goes down on the screen
        assertEquals(Joystick.UP, stickBits(0f, -80f, r))
        assertEquals(Joystick.DOWN, stickBits(0f, 80f, r))
        assertEquals(Joystick.UP or Joystick.RIGHT, stickBits(60f, -60f, r))
        assertEquals(Joystick.DOWN or Joystick.LEFT, stickBits(-60f, 60f, r))
        assertEquals(Joystick.DOWN or Joystick.RIGHT, stickBits(60f, 60f, r))
        assertEquals(Joystick.UP or Joystick.LEFT, stickBits(-60f, -60f, r))
    }
}

class SettingsTest {
    @Test
    fun aFrenchPhoneGetsTheFrenchRomsOthersTheEnglishOnes() {
        for (tag in listOf("fr-FR", "fr-CA", "fr-BE", "fr")) {
            assertEquals(tag, "fr", Settings.languageFor(Locale.forLanguageTag(tag)))
        }
        for (tag in listOf("en-US", "en-GB", "de-DE", "es-ES", "it-IT", "ja-JP")) {
            assertEquals(tag, "en", Settings.languageFor(Locale.forLanguageTag(tag)))
        }
    }

    @Test
    fun theLanguageFollowsThePhoneUnlessChosen() {
        val auto = Settings()
        assertEquals("fr", auto.language(Locale.FRANCE))
        assertEquals("en", auto.language(Locale.GERMANY))
        assertEquals("en", auto.copy(languageChoice = "en").language(Locale.FRANCE))
        assertEquals("fr", auto.copy(languageChoice = "fr").language(Locale.US))
    }

    @Test
    fun theSettingsAreSavedAndReadBack() {
        val s = Settings(model = "464", languageChoice = "fr", ctm = false, playcity = true, vibrate = false)
        assertEquals(s, Settings.fromPrefsJson(s.toPrefsJson()))
        // the settings of the first version (a "language" chosen by hand, "display")
        val old = Settings.fromPrefsJson("""{"model":"6128","language":"fr","display":"raw"}""")
        assertNotNull(old)
        assertEquals("fr", old!!.languageChoice)
        assertEquals(false, old.ctm)
    }

    @Test
    fun theEngineGetsOnlyTheKeysItKnows() {
        // rust/src/config.rs refuses unknown fields
        val keys = org.json.JSONObject(Settings().toEngineJson(Locale.US)).keys().asSequence().toSet()
        assertEquals(
            setOf("model", "language", "display", "m4", "m4Wifi", "rulezcharge", "unidos", "playcity", "autorun", "audio"),
            keys,
        )
    }
}

class KeyboardTest {
    private val legends = parseKeyLegends(
        // what Native.keyLegends gives: line = the key's number
        List(80) { "" }.toMutableList().apply {
            this[69] = "a\tA"
            this[64] = "&\t1"
            this[65] = "é\t2"
        }.joinToString("\n"),
    )

    @Test
    fun theKeycapsShowTheFirmwaresLegends() {
        assertEquals("A", keycapText(CpcKey("Q", 69), legends))
        assertEquals("1\n&", keycapText(CpcKey("1", 64), legends))
        assertEquals("2\né", keycapText(CpcKey("2", 65), legends))
        // the named keys keep their names; a key without a legend its label
        assertEquals("RET", keycapText(CpcKey("RET", 18), legends))
        assertEquals("Z", keycapText(CpcKey("Z", 71), legends))
    }

    @Test
    fun theAutorunTextIsShownPlainly() {
        assertEquals("RUN\"DISC", MainActivity.plain("RUN\"DISC\\(RET)"))
        assertEquals("|TAPE RUN\"", MainActivity.plain("|TAPE\\(RET)RUN\"\\(RET)\\(WAIT 1)\\(RET)"))
    }
}
