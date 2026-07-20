package com.gpxanimator.mobile.locale

import org.junit.Assert.assertEquals
import org.junit.Test

class AppLanguageTest {
    @Test
    fun knownLanguageTagsMapToSupportedChoices() {
        assertEquals(AppLanguage.English, AppLanguage.fromLanguageTag("en"))
        assertEquals(AppLanguage.English, AppLanguage.fromLanguageTag("en-US"))
        assertEquals(AppLanguage.TraditionalChinese, AppLanguage.fromLanguageTag("zh-TW"))
        assertEquals(AppLanguage.TraditionalChinese, AppLanguage.fromLanguageTag("zh-Hant"))
    }

    @Test
    fun unsupportedOrEmptyLanguageTagsUseSystemDefault() {
        assertEquals(AppLanguage.SystemDefault, AppLanguage.fromLanguageTag(null))
        assertEquals(AppLanguage.SystemDefault, AppLanguage.fromLanguageTag(""))
        assertEquals(AppLanguage.SystemDefault, AppLanguage.fromLanguageTag("ja-JP"))
    }
}
