// ============================================================================
// i18n.js - Internationalization Module for EPG Viewer
// ============================================================================

import { invoke, hasTauri, setWindowTitle } from "./state.js";

let currentLang = "hungarian";
let baseDict = {};     // English dictionary (universal fallback)
let currentDict = {};  // Active dictionary merged on top of baseDict
const loadedLanguagesCache = {};
const languageChangeListeners = [];

export function getCurrentLang() {
  return currentLang;
}

export function setLanguageForTesting(lang, dict = {}, base = {}) {
  currentLang = lang;
  currentDict = { ...dict };
  baseDict = { ...base };
}

// Emergency minimal fallback dictionary in case of complete disk/db/network failure
const EMERGENCY_FALLBACK = {
  app_title: "EPG/XML TV Guide Viewer",
  btn_ok: "OK",
  btn_cancel: "Cancel",
  alert_info_title: "Information",
  alert_error_title: "Error occurred",
  status_error: "Error processing data.",
  days: ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"],
  months: [
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December"
  ],
  logo_dedup_confirm: "Are you sure you want to merge duplicate logos and eliminate redundant links?",
  logo_dedup_optimizing: "Optimization in progress...",
  no_sources_available: "No sources available.",
  no_channels_in_source: "No channels found in this source.",
  no_channels_match_filter: "No channels match the filter."
};

/**
 * Fetch a language dictionary by code.
 * Multi-tier fallback strategy:
 *   1. Try local fetch from './languages/${lang}.lng'
 *   2. If fetch fails, try Tauri SQLite database via get_language_content
 *   3. If SQLite fails, try GitHub raw download
 *   4. Fall back to minimal emergency strings
 */
async function fetchLanguageDictionary(lang) {
  if (loadedLanguagesCache[lang]) {
    return loadedLanguagesCache[lang];
  }

  const tauriAvailable = hasTauri();

  // Tier 1: Try local asset fetch
  try {
    const res = await fetch(`./languages/${lang}.lng`);
    if (res.ok) {
      const data = await res.json();
      loadedLanguagesCache[lang] = data;
      return data;
    }
  } catch (err) {
    console.warn(`[i18n] Local fetch for ${lang}.lng failed, attempting recovery...`, err);
  }

  // Tier 2: Try SQLite database cache / embedded Rust fallback
  if (tauriAvailable) {
    try {
      const content = await invoke("get_language_content", { lang });
      if (content && typeof content === "string") {
        const data = JSON.parse(content);
        loadedLanguagesCache[lang] = data;
        return data;
      }
    } catch (err) {
      console.warn(`[i18n] SQLite get_language_content for ${lang} failed:`, err);
    }
  }

  // Tier 3: Try downloading from GitHub repository
  if (tauriAvailable) {
    try {
      const downloaded = await invoke("download_languages_from_github");
      if (downloaded && downloaded[lang]) {
        const data = JSON.parse(downloaded[lang]);
        loadedLanguagesCache[lang] = data;
        return data;
      }
    } catch (err) {
      console.warn(`[i18n] GitHub download recovery for ${lang} failed:`, err);
    }
  }

  // Tier 4: Minimal emergency fallback
  console.error(`[i18n] Could not load dictionary for language: ${lang}. Using emergency fallback.`);
  return { ...EMERGENCY_FALLBACK };
}

/**
 * Translates a given key with optional {param} replacements.
 * Falls back to baseDict (English), then raw key.
 */
export function t(key, params = {}) {
  let val =
    currentDict && currentDict[key] !== undefined
      ? currentDict[key]
      : baseDict && baseDict[key] !== undefined
        ? baseDict[key]
        : key;

  if (typeof val === "string" && params && typeof params === "object") {
    for (const [k, v] of Object.entries(params)) {
      val = val.split(`{${k}}`).join(String(v));
    }
  }
  return val;
}

/**
 * Apply all translations to current DOM elements with data-i18n attributes.
 */
export function applyTranslations() {
  const title = t("app_title");
  document.title = title;
  setWindowTitle(title).catch(() => {});

  document.querySelectorAll("[data-i18n]").forEach((el) => {
    const key = el.getAttribute("data-i18n");
    el.textContent = t(key);
  });

  document.querySelectorAll("[data-i18n-placeholder]").forEach((el) => {
    const key = el.getAttribute("data-i18n-placeholder");
    el.placeholder = t(key);
  });

  document.querySelectorAll("[data-i18n-title]").forEach((el) => {
    const key = el.getAttribute("data-i18n-title");
    el.title = t(key);
  });
}

/**
 * Register a callback invoked whenever the active language changes.
 */
export function addLanguageChangeListener(fn) {
  if (typeof fn === "function") {
    languageChangeListeners.push(fn);
  }
}

/**
 * Switch active language, load dictionaries, persist setting, and apply translations.
 */
export async function setLanguage(lang, options = {}) {
  const { persist = true, notify = true } = options;
  currentLang = lang;
  const select = document.getElementById("languageSelect");
  if (select && select.value !== lang) {
    select.value = lang;
  }

  // 1. Ensure English base dictionary is loaded for fallback
  if (!baseDict || Object.keys(baseDict).length === 0) {
    baseDict = await fetchLanguageDictionary("english");
  }

  // 2. Load requested language dictionary
  if (lang === "english") {
    currentDict = baseDict;
  } else {
    const langDict = await fetchLanguageDictionary(lang);
    currentDict = { ...baseDict, ...langDict };
  }

  // 3. Save setting in SQLite (only when persisted change is requested)
  if (persist && hasTauri()) {
    try {
      await invoke("set_setting", { key: "language", value: lang });
    } catch (e) {
      console.error("[i18n] Failed to persist language setting:", e);
    }
  }

  // 4. Update UI
  applyTranslations();

  // 5. Notify listeners
  if (notify) {
    for (const listener of languageChangeListeners) {
      try {
        listener(lang);
      } catch (e) {
        console.error("[i18n] Error in language change listener:", e);
      }
    }
  }
}
