package com.gpxanimator.mobile.locale

import android.app.LocaleManager
import android.content.Context
import android.content.res.Configuration
import android.os.Build
import android.os.LocaleList
import androidx.annotation.StringRes
import androidx.core.content.edit
import com.gpxanimator.mobile.R
import java.util.Locale

enum class AppLanguage(
    val languageTag: String?,
    @get:StringRes val labelRes: Int,
) {
    SystemDefault(null, R.string.language_system),
    English("en", R.string.language_english),
    TraditionalChinese("zh-TW", R.string.language_traditional_chinese),
    ;

    companion object {
        fun fromLanguageTag(languageTag: String?): AppLanguage =
            when {
                languageTag.isNullOrBlank() -> SystemDefault
                languageTag.startsWith("en", ignoreCase = true) -> English
                languageTag.startsWith("zh-Hant", ignoreCase = true) ||
                    languageTag.equals("zh-TW", ignoreCase = true) -> TraditionalChinese

                else -> SystemDefault
            }
    }
}

object AppLanguagePreferences {
    private const val PREFERENCES_NAME = "app-language"
    private const val KEY_LANGUAGE_TAG = "language-tag"

    fun current(context: Context): AppLanguage {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            val locales = context.getSystemService(LocaleManager::class.java).applicationLocales
            if (!locales.isEmpty) {
                return AppLanguage.fromLanguageTag(locales[0].toLanguageTag())
            }
        }
        return AppLanguage.fromLanguageTag(
            context.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)
                .getString(KEY_LANGUAGE_TAG, null),
        )
    }

    fun set(context: Context, language: AppLanguage) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            val locales =
                language.languageTag?.let(LocaleList::forLanguageTags)
                    ?: LocaleList.getEmptyLocaleList()
            context.getSystemService(LocaleManager::class.java).applicationLocales = locales
            return
        }

        context.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE).edit {
            if (language.languageTag == null) {
                remove(KEY_LANGUAGE_TAG)
            } else {
                putString(KEY_LANGUAGE_TAG, language.languageTag)
            }
        }
    }

    fun localizedContext(base: Context): Context {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            return base
        }
        val language = current(base)
        val languageTag = language.languageTag ?: return base
        val locale = Locale.forLanguageTag(languageTag)
        val configuration = Configuration(base.resources.configuration)
        configuration.setLocale(locale)
        configuration.setLocales(LocaleList(locale))
        return base.createConfigurationContext(configuration)
    }
}
