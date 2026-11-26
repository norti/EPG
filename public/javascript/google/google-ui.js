/**
 * Google OAuth 2.0, Drive Sync & Calendar UI Controller
 */
import { state, invoke, listen, hasTauri, setFavoritesList } from "../application/state.js";
import {
  t,
  getCurrentLang,
  setLanguage,
  addLanguageChangeListener,
} from "../application/i18n.js";
import { Icons } from "../utils/icons.js";
import {
  iconHtml,
  showConfirm,
  showAlert,
  setExportImportStatus,
} from "../application/ui.js";
import { refreshSourcesFromDb } from "../application/sources.js";
import {
  fetchEPG,
  invalidateEpgSourceCache,
  renderEPG,
} from "../application/epg.js";
import { refreshFavoritesModalIfOpen } from "../application/favorites.js";
import {
  invalidateChannelOrderCache,
  renderChannelOrderList,
} from "../application/channels.js";
import { updateModalLeftColVisibility } from "../tmdb/tmdb.js";
import {
  dateToLocalIsoWithOffset,
  buildCalendarSummary,
  normalizeCalendarChannelId,
  buildCalendarEpgKey,
  getGoogleCalendarIconSvg,
} from "./google-calendar-utils.js";

let authStatus = { loggedIn: false, configured: false, profile: null };
let syncState = {
  lastDriveSaveAt: null,
  lastDriveLoadAt: null,
  lastRemoteModifiedAt: null,
  autoSyncEnabled: false,
  targetCalendarId: "primary",
};
let cachedCalendars = [];
let calendarEventKeySet = new Set();
let trackedEventsCount = 0;
let newerRemoteAvailable = false;
let loginInProgress = false;
let cachedAvatarUrl = null;
let cachedAvatarSrc = null;
let modalCalendarSeq = 0;

export function isGoogleLoggedIn() {
  return !!authStatus.loggedIn;
}

export async function prepareCalendarMatchesForEpg() {
  if (!hasTauri() || !authStatus.loggedIn) {
    calendarEventKeySet = new Set();
    trackedEventsCount = 0;
    return;
  }
  try {
    const keys = await invoke("google_get_calendar_event_keys");
    calendarEventKeySet = new Set(Array.isArray(keys) ? keys : []);
    trackedEventsCount = calendarEventKeySet.size;
  } catch (e) {
    console.warn("[Google] prepareCalendarMatchesForEpg error:", e);
  }
}

export function isOnGoogleCalendar(epgKey) {
  if (!authStatus.loggedIn || !epgKey) return false;
  return calendarEventKeySet.has(epgKey);
}

function renderDynamicCalendarIcons() {
  const today = new Date().getDate();
  const cardIcon = document.getElementById("googleCalendarCardIcon");
  if (cardIcon) {
    cardIcon.innerHTML = getGoogleCalendarIconSvg(today, 22);
  }
  const modalIcon = document.getElementById("modalCalendarBrandIcon");
  if (modalIcon) {
    modalIcon.innerHTML = getGoogleCalendarIconSvg(today, 16);
  }
}

function renderTrackedEventsCount() {
  const el = document.getElementById("googleCalendarTrackedCount");
  if (!el) return;
  el.textContent = t("google_calendar_tracked_count", {
    count: trackedEventsCount,
  });
}

async function refreshTrackedEventsCount() {
  if (!hasTauri() || !authStatus.loggedIn) {
    trackedEventsCount = 0;
    renderTrackedEventsCount();
    return;
  }
  try {
    const count = await invoke("google_get_calendar_events_count");
    trackedEventsCount = typeof count === "number" ? count : 0;
  } catch (_) {}
  renderTrackedEventsCount();
}

export function formatGoogleError(err) {
  const s = String(err || "");
  if (s.includes("GOOGLE_NOT_CONFIGURED")) return t("google_not_configured");
  if (s.includes("GOOGLE_SESSION_EXPIRED")) return t("google_session_expired");
  if (s.includes("GOOGLE_CALENDAR_NO_PERMISSION"))
    return t("google_calendar_no_permission");
  if (s.includes("GOOGLE_CALENDAR_NOT_FOUND"))
    return t("google_calendar_not_found");
  if (s.includes("GOOGLE_DRIVE_NO_BACKUP")) return t("google_drive_no_backup");
  if (s.includes("GOOGLE_OFFLINE")) return t("google_offline_error");
  if (s.includes("google_login_timeout")) return t("google_login_timeout");
  return s;
}

function formatLocaleDate(isoStr) {
  if (!isoStr) return "";
  const d = new Date(isoStr);
  if (isNaN(d.getTime())) return isoStr;
  const locale = getCurrentLang() === "english" ? "en-US" : "hu-HU";
  return d.toLocaleString(locale);
}

function resetLoginUi() {
  loginInProgress = false;
  const loginBtn = document.getElementById("googleLoginBtn");
  const cancelBtn = document.getElementById("googleCancelLoginBtn");
  if (loginBtn) loginBtn.disabled = false;
  if (cancelBtn) cancelBtn.classList.add("u-hidden");
}

function renderAvatarFallback(wrap) {
  const img = document.createElement("img");
  img.className = "google-avatar-fallback";
  img.alt = "Google";
  img.src = "images/google/google-g.svg";
  wrap.replaceChildren(img);
}

function renderAvatar(pictureUrl) {
  const wrap = document.getElementById("googleAvatarWrap");
  if (!wrap) return;
  if (!pictureUrl) {
    renderAvatarFallback(wrap);
    return;
  }
  if (cachedAvatarUrl === pictureUrl && cachedAvatarSrc) {
    const img = document.createElement("img");
    img.className = "google-avatar";
    img.alt = "";
    img.src = cachedAvatarSrc;
    wrap.replaceChildren(img);
    return;
  }
  renderAvatarFallback(wrap);
  invoke("cache_logo", { url: pictureUrl, imageType: "avatar" })
    .then((src) => {
      if (!src) return;
      cachedAvatarUrl = pictureUrl;
      cachedAvatarSrc = src;
      if (wrap.isConnected && authStatus.loggedIn) {
        const img = document.createElement("img");
        img.className = "google-avatar";
        img.alt = "";
        img.src = src;
        wrap.replaceChildren(img);
      }
    })
    .catch(() => {});
}

function renderAccountSection() {
  const loggedInEl = document.getElementById("googleLoggedIn");
  const loggedOutEl = document.getElementById("googleLoggedOut");
  const driveCard = document.getElementById("googleDriveCard");
  const calCard = document.getElementById("googleCalendarCard");
  if (!loggedInEl || !loggedOutEl) return;

  const isIn = !!authStatus.loggedIn;
  loggedInEl.classList.toggle("u-hidden", !isIn);
  loggedOutEl.classList.toggle("u-hidden", isIn);
  if (driveCard) driveCard.classList.toggle("u-hidden", !isIn);
  if (calCard) calCard.classList.toggle("u-hidden", !isIn);

  if (isIn) {
    const prof = authStatus.profile || {};
    const nameEl = document.getElementById("googleUserName");
    const emailEl = document.getElementById("googleUserEmail");
    if (nameEl) nameEl.textContent = prof.name || prof.email || "Google";
    if (emailEl) emailEl.textContent = prof.email || "";
    renderAvatar(prof.picture || null);
  }
}

function renderSyncBanner() {
  const banner = document.getElementById("googleNewerRemoteBanner");
  if (banner) {
    banner.classList.toggle("u-hidden", !newerRemoteAvailable);
  }
}

function renderDriveSection() {
  const label = document.getElementById("googleLastSaveLabel");
  if (label) {
    if (syncState.lastDriveSaveAt) {
      label.textContent = t("google_last_saved_at", {
        date: formatLocaleDate(syncState.lastDriveSaveAt),
      });
    } else {
      label.textContent = t("google_never_saved");
    }
  }
  const autoToggle = document.getElementById("googleAutoSyncToggle");
  if (autoToggle) {
    autoToggle.checked = !!syncState.autoSyncEnabled;
  }
  renderSyncBanner();
}

function formatCalendarLabel(cal) {
  return cal.primary
    ? `${cal.summary} (${t("google_calendar_primary")})`
    : cal.summary;
}

function getSelectedCalendarEntry() {
  if (!cachedCalendars.length) return null;
  const targetId = syncState.targetCalendarId || "primary";
  if (targetId === "primary") {
    return cachedCalendars.find((c) => c.primary) || cachedCalendars[0];
  }
  return (
    cachedCalendars.find((c) => c.id === targetId) ||
    cachedCalendars.find((c) => c.primary) ||
    cachedCalendars[0]
  );
}

function applyDotColor(dotEl, color) {
  if (!dotEl) return;
  if (color) {
    dotEl.style.setProperty("background-color", color, "important");
    dotEl.classList.remove("u-hidden");
  } else {
    dotEl.style.removeProperty("background-color");
    dotEl.classList.add("u-hidden");
  }
}

function closeCalendarDropdown() {
  const menu = document.getElementById("googleCalendarDropdownMenu");
  const btn = document.getElementById("googleCalendarDropdownBtn");
  if (menu) menu.classList.add("u-hidden");
  if (btn) btn.setAttribute("aria-expanded", "false");
}

function toggleCalendarDropdown() {
  const menu = document.getElementById("googleCalendarDropdownMenu");
  const btn = document.getElementById("googleCalendarDropdownBtn");
  if (!menu || !btn || btn.disabled) return;
  const willOpen = menu.classList.contains("u-hidden");
  menu.classList.toggle("u-hidden", !willOpen);
  btn.setAttribute("aria-expanded", willOpen ? "true" : "false");
}

function renderCalendarOptions() {
  const dot = document.getElementById("googleCalendarColorDot");
  const labelEl = document.getElementById("googleCalendarSelectedLabel");
  const menu = document.getElementById("googleCalendarDropdownMenu");
  if (!labelEl || !menu) return;

  menu.innerHTML = "";

  if (!cachedCalendars.length) {
    labelEl.textContent = t("google_calendar_primary");
    applyDotColor(dot, "");
    return;
  }

  const targetId = syncState.targetCalendarId || "primary";
  if (targetId !== "primary" && !cachedCalendars.some((c) => c.id === targetId)) {
    syncState.targetCalendarId = "primary";
    invoke("google_set_target_calendar", { calendarId: "primary" }).catch(
      () => {},
    );
  }

  const selectedCal = getSelectedCalendarEntry();
  if (selectedCal) {
    labelEl.textContent = formatCalendarLabel(selectedCal);
    applyDotColor(dot, selectedCal.backgroundColor || "");
  }

  for (const cal of cachedCalendars) {
    const isSelected = selectedCal && cal.id === selectedCal.id;
    const itemBtn = document.createElement("button");
    itemBtn.type = "button";
    itemBtn.className = `google-calendar-option${isSelected ? " is-selected" : ""}`;
    itemBtn.setAttribute("role", "option");
    itemBtn.setAttribute("aria-selected", isSelected ? "true" : "false");

    const itemDot = document.createElement("span");
    itemDot.className = "google-calendar-color-dot";
    applyDotColor(itemDot, cal.backgroundColor || "#4285f4");

    const itemLabel = document.createElement("span");
    itemLabel.className = "google-calendar-option-label";
    itemLabel.textContent = formatCalendarLabel(cal);

    itemBtn.appendChild(itemDot);
    itemBtn.appendChild(itemLabel);

    if (isSelected) {
      const checkEl = document.createElement("span");
      checkEl.className = "google-calendar-option-check";
      checkEl.innerHTML = iconHtml("check", 14);
      itemBtn.appendChild(checkEl);
    }

    itemBtn.onclick = () => selectCalendarEntry(cal);
    menu.appendChild(itemBtn);
  }
}

async function loadCalendarList(forceRefresh = false) {
  const triggerBtn = document.getElementById("googleCalendarDropdownBtn");
  const refreshBtn = document.getElementById("googleCalendarRefreshBtn");
  if (!triggerBtn || !authStatus.loggedIn) return;

  closeCalendarDropdown();
  triggerBtn.disabled = true;
  if (refreshBtn) refreshBtn.disabled = true;
  setExportImportStatus("googleCalendarStatus", t("google_calendar_loading"), "loading");

  try {
    cachedCalendars = await invoke("google_list_calendars", { forceRefresh });
    renderCalendarOptions();
    const statusEl = document.getElementById("googleCalendarStatus");
    if (statusEl) statusEl.classList.add("u-hidden");
  } catch (e) {
    const msg = formatGoogleError(e);
    setExportImportStatus(
      "googleCalendarStatus",
      t("google_calendar_load_error", { error: msg }),
      "error",
    );
    if (String(e).includes("GOOGLE_SESSION_EXPIRED")) {
      authStatus.loggedIn = false;
      authStatus.profile = null;
      renderAccountSection();
    }
  } finally {
    triggerBtn.disabled = false;
    if (refreshBtn) refreshBtn.disabled = false;
  }
}

async function selectCalendarEntry(cal) {
  if (!cal) return;
  const prevTargetId = syncState.targetCalendarId;
  const calendarIdToSave = cal.primary ? "primary" : cal.id;
  syncState.targetCalendarId = calendarIdToSave;
  renderCalendarOptions();
  closeCalendarDropdown();

  try {
    await invoke("google_set_target_calendar", {
      calendarId: calendarIdToSave,
    });
    const statusEl = document.getElementById("googleCalendarStatus");
    if (statusEl) statusEl.classList.add("u-hidden");
  } catch (err) {
    syncState.targetCalendarId = prevTargetId;
    renderCalendarOptions();
    const msg = formatGoogleError(err);
    await showAlert(
      t("alert_error_title"),
      t("google_calendar_set_error", { error: msg }),
    );
    await loadCalendarList(true);
  }
}

async function startLogin() {
  if (loginInProgress) return;
  loginInProgress = true;
  const loginBtn = document.getElementById("googleLoginBtn");
  const cancelBtn = document.getElementById("googleCancelLoginBtn");
  if (loginBtn) loginBtn.disabled = true;
  if (cancelBtn) cancelBtn.classList.remove("u-hidden");
  setExportImportStatus("googleAuthInlineStatus", t("google_login_pending"), "loading");

  try {
    const startData = await invoke("google_start_auth");
    if (!startData.opened && startData.authUrl) {
      await invoke("open_url", { url: startData.authUrl });
    }
  } catch (e) {
    resetLoginUi();
    const msg = formatGoogleError(e);
    setExportImportStatus("googleAuthInlineStatus", msg, "error");
    await showAlert(
      t("alert_error_title"),
      t("google_login_error", { error: msg }),
    );
  }
}

async function cancelLogin() {
  await invoke("google_cancel_auth").catch(() => {});
  resetLoginUi();
  const statusEl = document.getElementById("googleAuthInlineStatus");
  if (statusEl) statusEl.classList.add("u-hidden");
}

async function handleAuthEvent(payload) {
  resetLoginUi();
  const statusEl = document.getElementById("googleAuthInlineStatus");

  switch (payload?.status) {
    case "ok": {
      if (statusEl) statusEl.classList.add("u-hidden");
      try {
        authStatus = await invoke("google_get_auth_status");
        syncState = await invoke("google_get_sync_state");
      } catch (_) {
        authStatus = {
          loggedIn: true,
          configured: true,
          profile: payload.profile || null,
        };
      }
      renderAccountSection();
      renderDriveSection();
      await loadCalendarList(true);
      await invoke("google_sync_calendar_events", { forceFull: true }).catch(
        () => {},
      );
      await prepareCalendarMatchesForEpg();
      renderTrackedEventsCount();
      renderEPG();
      await showAlert(t("alert_success_title"), t("google_login_success"));
      break;
    }
    case "error": {
      const msg = formatGoogleError(payload.message);
      setExportImportStatus("googleAuthInlineStatus", msg, "error");
      await showAlert(
        t("alert_error_title"),
        t("google_login_failed", { error: msg }),
      );
      break;
    }
    case "cancelled": {
      if (statusEl) statusEl.classList.add("u-hidden");
      break;
    }
    default:
      break;
  }
}

async function logout() {
  const confirmed = await showConfirm(
    t("google_logout_confirm_title"),
    t("google_logout_confirm_body"),
  );
  if (!confirmed) return;

  try {
    await invoke("google_logout");
    authStatus.loggedIn = false;
    authStatus.profile = null;
    cachedCalendars = [];
    calendarEventKeySet = new Set();
    trackedEventsCount = 0;
    newerRemoteAvailable = false;
    cachedAvatarUrl = null;
    cachedAvatarSrc = null;
    syncState = {
      ...syncState,
      lastDriveSaveAt: null,
      lastDriveLoadAt: null,
      lastRemoteModifiedAt: null,
    };
    renderAccountSection();
    renderDriveSection();
    renderTrackedEventsCount();
    renderEPG();
    await showAlert(t("alert_success_title"), t("google_logout_success"));
  } catch (e) {
    await showAlert(t("alert_error_title"), formatGoogleError(e));
  }
}

async function saveToDrive() {
  const btn = document.getElementById("googleSaveBtn");
  if (btn) btn.disabled = true;
  setExportImportStatus("googleDriveStatus", t("google_saving"), "loading");

  try {
    await invoke("google_save_to_drive");
    newerRemoteAvailable = false;
    syncState = await invoke("google_get_sync_state");
    renderDriveSection();
    setExportImportStatus("googleDriveStatus", t("google_save_success"), "success");
  } catch (e) {
    const msg = formatGoogleError(e);
    setExportImportStatus("googleDriveStatus", msg, "error");
    await showAlert(
      t("alert_error_title"),
      t("google_save_error", { error: msg }),
    );
  } finally {
    if (btn) btn.disabled = false;
  }
}

async function loadFromDrive() {
  const btn = document.getElementById("googleLoadBtn");
  if (btn) btn.disabled = true;
  setExportImportStatus("googleDriveStatus", t("status_loading"), "loading");

  try {
    const preview = await invoke("google_load_from_drive");
    const confirmed = await showConfirm(
      t("google_load_confirm_title"),
      t("google_load_confirm_body", {
        date: formatLocaleDate(preview.timestamp),
        client: preview.clientName,
        settings: preview.settingsCount,
        sources: preview.sourcesCount,
        favorites: preview.favoritesCount,
        presets: preview.presetsCount,
      }),
    );
    if (!confirmed) {
      const statusEl = document.getElementById("googleDriveStatus");
      if (statusEl) statusEl.classList.add("u-hidden");
      return;
    }

    await invoke("google_apply_drive_data");
    newerRemoteAvailable = false;
    syncState = await invoke("google_get_sync_state");
    renderDriveSection();
    setExportImportStatus("googleDriveStatus", t("google_load_success"), "success");
    await showAlert(t("alert_success_title"), t("google_load_success"));

    // Clean in-app reload matching settings.js import flow (no location.reload)
    invalidateChannelOrderCache();
    invalidateEpgSourceCache();
    await refreshSourcesFromDb();
    setFavoritesList(await invoke("get_favorites"));
    state.settings = await invoke("get_settings");
    await setLanguage(state.settings.language || "hungarian", {
      persist: false,
      notify: true,
    });
    await fetchEPG({ forceReload: true });
    refreshFavoritesModalIfOpen();
    await renderChannelOrderList();
  } catch (e) {
    const msg = formatGoogleError(e);
    setExportImportStatus("googleDriveStatus", msg, "error");
    await showAlert(
      t("alert_error_title"),
      t("google_load_error", { error: msg }),
    );
  } finally {
    if (btn) btn.disabled = false;
  }
}

async function toggleAutoSync(e) {
  const enabled = !!e.target.checked;
  try {
    await invoke("google_set_auto_sync", { enabled });
    syncState.autoSyncEnabled = enabled;
  } catch (err) {
    e.target.checked = !enabled;
    await showAlert(t("alert_error_title"), formatGoogleError(err));
  }
}

async function runCalendarCleanup() {
  const modeEl = document.getElementById("googleCalendarCleanupMode");
  const daysInput = document.getElementById("googleCalendarCleanupDays");
  const btn = document.getElementById("googleCalendarCleanupBtn");
  if (!modeEl || !btn) return;

  const mode = modeEl.value;
  let olderThanDays = null;
  if (mode === "custom") {
    const parsed = parseInt(daysInput?.value || "30", 10);
    olderThanDays = isNaN(parsed) || parsed < 1 ? 1 : parsed;
    if (daysInput) daysInput.value = String(olderThanDays);
  } else if (mode !== "all") {
    olderThanDays = parseInt(mode, 10) || 30;
  }

  const confirmBody =
    olderThanDays === null
      ? t("google_calendar_cleanup_confirm_all")
      : t("google_calendar_cleanup_confirm_days", { days: olderThanDays });

  const confirmed = await showConfirm(
    t("google_calendar_cleanup_confirm_title"),
    confirmBody,
  );
  if (!confirmed) return;

  btn.disabled = true;
  setExportImportStatus(
    "googleCalendarCleanupStatus",
    t("google_calendar_cleanup_running"),
    "loading",
  );

  try {
    const res = await invoke("google_cleanup_calendar_events", {
      olderThanDays,
    });
    await prepareCalendarMatchesForEpg();
    trackedEventsCount =
      typeof res.remainingCount === "number"
        ? res.remainingCount
        : calendarEventKeySet.size;
    renderTrackedEventsCount();
    renderEPG();

    if (res.deletedCount > 0) {
      setExportImportStatus(
        "googleCalendarCleanupStatus",
        t("google_calendar_cleanup_done", {
          deleted: res.deletedCount,
          remaining: res.remainingCount,
        }),
        "success",
      );
    } else {
      setExportImportStatus(
        "googleCalendarCleanupStatus",
        t("google_calendar_cleanup_none"),
        "success",
      );
    }
  } catch (e) {
    const msg = formatGoogleError(e);
    setExportImportStatus(
      "googleCalendarCleanupStatus",
      t("google_calendar_cleanup_error", { error: msg }),
      "error",
    );
    await showAlert(
      t("alert_error_title"),
      t("google_calendar_cleanup_error", { error: msg }),
    );
  } finally {
    btn.disabled = false;
  }
}

function bindSettingsEvents() {
  const loginBtn = document.getElementById("googleLoginBtn");
  const cancelBtn = document.getElementById("googleCancelLoginBtn");
  const logoutBtn = document.getElementById("googleLogoutBtn");
  const saveBtn = document.getElementById("googleSaveBtn");
  const loadBtn = document.getElementById("googleLoadBtn");
  const autoToggle = document.getElementById("googleAutoSyncToggle");
  const calDropdownBtn = document.getElementById("googleCalendarDropdownBtn");
  const calRefreshBtn = document.getElementById("googleCalendarRefreshBtn");
  const cleanupMode = document.getElementById("googleCalendarCleanupMode");
  const cleanupCustomWrap = document.getElementById(
    "googleCalendarCleanupCustomWrap",
  );
  const cleanupBtn = document.getElementById("googleCalendarCleanupBtn");

  if (loginBtn) loginBtn.onclick = startLogin;
  if (cancelBtn) cancelBtn.onclick = cancelLogin;
  if (logoutBtn) logoutBtn.onclick = logout;
  if (saveBtn) saveBtn.onclick = saveToDrive;
  if (loadBtn) loadBtn.onclick = loadFromDrive;
  if (autoToggle) autoToggle.onchange = toggleAutoSync;
  if (calDropdownBtn) {
    calDropdownBtn.onclick = (e) => {
      e.stopPropagation();
      toggleCalendarDropdown();
    };
  }
  if (calRefreshBtn) {
    calRefreshBtn.onclick = async () => {
      await loadCalendarList(true);
      try {
        await invoke("google_sync_calendar_events", { forceFull: false });
        await prepareCalendarMatchesForEpg();
        renderTrackedEventsCount();
        renderEPG();
      } catch (_) {}
    };
  }
  if (cleanupMode && cleanupCustomWrap) {
    cleanupMode.onchange = () => {
      cleanupCustomWrap.classList.toggle(
        "u-hidden",
        cleanupMode.value !== "custom",
      );
    };
  }
  if (cleanupBtn) cleanupBtn.onclick = runCalendarCleanup;

  document.addEventListener("click", (e) => {
    const dropdown = document.getElementById("googleCalendarDropdown");
    if (dropdown && !dropdown.contains(e.target)) {
      closeCalendarDropdown();
    }
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      closeCalendarDropdown();
    }
  });
}

export async function loadGoogleSettingsTab() {
  if (!hasTauri()) return;
  renderDynamicCalendarIcons();
  try {
    authStatus = await invoke("google_get_auth_status");
    if (authStatus.loggedIn) {
      syncState = await invoke("google_get_sync_state");
    }
  } catch (e) {
    console.warn("[Google] loadGoogleSettingsTab error:", e);
  }
  renderAccountSection();
  renderDriveSection();
  if (authStatus.loggedIn) {
    await Promise.all([loadCalendarList(false), refreshTrackedEventsCount()]);
  }
}

export async function initGoogleUI() {
  renderDynamicCalendarIcons();
  if (!hasTauri()) return;

  try {
    authStatus = await invoke("google_get_auth_status");
    if (authStatus.loggedIn) {
      syncState = await invoke("google_get_sync_state");
      await prepareCalendarMatchesForEpg();
    }
  } catch (e) {
    console.warn("[Google] init status error:", e);
  }

  bindSettingsEvents();

  await listen("google-auth-state", async (event) => {
    await handleAuthEvent(event.payload);
  });

  await listen("google-sync-state", (event) => {
    if (event.payload?.status === "newerRemoteAvailable") {
      newerRemoteAvailable = true;
      renderSyncBanner();
    }
  });

  await listen("google-calendar-events-updated", async () => {
    await prepareCalendarMatchesForEpg();
    renderTrackedEventsCount();
    renderEPG();
  });

  addLanguageChangeListener(() => {
    if (document.getElementById("tabPaneGoogle")?.classList.contains("active")) {
      renderDriveSection();
      renderTrackedEventsCount();
      if (authStatus.loggedIn) {
        renderCalendarOptions();
      }
    }
  });
}

// ============================================================================
// Programme Detail Modal – Google Calendar Button (Left Column, below Watchlist)
// ============================================================================

export function detachGoogleCalendarFromModal() {
  modalCalendarSeq++;
  const section = document.getElementById("modalGoogleCalendarSection");
  const btn = document.getElementById("modalCalendarBtn");
  const btnText = document.getElementById("modalCalendarBtnText");
  const linkBtn = document.getElementById("modalCalendarLinkBtn");
  const targetName = document.getElementById("modalCalendarTargetName");

  if (section) section.classList.add("u-hidden");
  if (btn) {
    btn.className = "btn btn-blue google-modal-cal-btn";
    btn.disabled = false;
    btn.onclick = null;
  }
  if (btnText) btnText.textContent = t("google_calendar_add_btn");
  if (linkBtn) {
    linkBtn.classList.add("u-hidden");
    linkBtn.onclick = null;
  }
  if (targetName) {
    targetName.classList.add("u-hidden");
    targetName.replaceChildren();
  }
  updateModalLeftColVisibility();
}

function renderModalTargetCalendarBadge(calId, fallbackName = null) {
  const nameEl = document.getElementById("modalCalendarTargetName");
  if (!nameEl) return;
  nameEl.classList.add("u-hidden");
  nameEl.replaceChildren();

  if (!calId || calId === "primary") return;

  const cal = cachedCalendars.find((c) => c.id === calId);
  const label = cal?.summary || fallbackName;
  if (!label || cal?.primary) return;

  if (cal?.backgroundColor) {
    const dot = document.createElement("span");
    dot.className = "google-calendar-color-dot";
    applyDotColor(dot, cal.backgroundColor);
    nameEl.appendChild(dot);
  }
  const txt = document.createElement("span");
  txt.textContent = label;
  nameEl.appendChild(txt);
  nameEl.title = label;
  nameEl.classList.remove("u-hidden");
}

async function ensureCalendarsLoadedForModal(seq) {
  if (!cachedCalendars.length) {
    try {
      cachedCalendars = await invoke("google_list_calendars", {
        forceRefresh: false,
      });
    } catch (_) {}
  }
  return seq === modalCalendarSeq;
}

export function attachGoogleCalendarToModal({ title, desc, airing, tmdbData }) {
  detachGoogleCalendarFromModal();
  const seq = modalCalendarSeq;

  const section = document.getElementById("modalGoogleCalendarSection");
  const btn = document.getElementById("modalCalendarBtn");
  const btnText = document.getElementById("modalCalendarBtnText");
  const linkBtn = document.getElementById("modalCalendarLinkBtn");

  if (
    !section ||
    !btn ||
    !authStatus.loggedIn ||
    !airing ||
    !(airing.startDate instanceof Date) ||
    isNaN(airing.startDate.getTime()) ||
    !(airing.stopDate instanceof Date) ||
    isNaN(airing.stopDate.getTime())
  ) {
    return;
  }

  renderDynamicCalendarIcons();

  const sourceId = String(airing.sourceId || "").trim();
  const channelId = normalizeCalendarChannelId(sourceId, airing.channelId || "");
  const epgKey = buildCalendarEpgKey(sourceId, channelId, airing.startDate);
  const startTs = Math.floor(airing.startDate.getTime() / 1000);
  const stopTs = Math.floor(airing.stopDate.getTime() / 1000);

  let trackedEvent = null;

  const applyButtonMode = () => {
    if (trackedEvent) {
      btn.className = "btn google-modal-cal-btn is-on";
      btn.disabled = false;
      if (btnText) btnText.textContent = t("google_calendar_remove_btn");
      renderModalTargetCalendarBadge(
        trackedEvent.calendarId,
        trackedEvent.calendarName,
      );
      if (linkBtn) {
        const hasLink = !!trackedEvent.htmlLink;
        linkBtn.classList.toggle("u-hidden", !hasLink);
        Icons.replace(linkBtn);
        linkBtn.onclick = () => {
          if (trackedEvent?.htmlLink) {
            invoke("open_url", { url: trackedEvent.htmlLink }).catch(() => {});
          }
        };
      }
    } else {
      btn.className = "btn btn-blue google-modal-cal-btn";
      btn.disabled = false;
      if (btnText) btnText.textContent = t("google_calendar_add_btn");
      renderModalTargetCalendarBadge(syncState.targetCalendarId || "primary");
      if (linkBtn) {
        linkBtn.classList.add("u-hidden");
        linkBtn.onclick = null;
      }
    }
  };

  section.classList.remove("u-hidden");
  applyButtonMode();
  updateModalLeftColVisibility();

  // Check local DB immediately to know if this programme is already in Google Calendar
  (async () => {
    try {
      const [freshSync, existing] = await Promise.all([
        invoke("google_get_sync_state"),
        epgKey
          ? invoke("google_get_calendar_event", { epgKey })
          : Promise.resolve(null),
      ]);
      if (seq !== modalCalendarSeq) return;
      syncState = freshSync;
      trackedEvent = existing || null;
      await ensureCalendarsLoadedForModal(seq);
      if (seq !== modalCalendarSeq) return;
      applyButtonMode();
    } catch (_) {}
  })();

  btn.onclick = async () => {
    if (trackedEvent) {
      // Remove existing calendar event
      btn.disabled = true;
      if (btnText) btnText.textContent = t("google_calendar_removing");
      try {
        await invoke("google_remove_calendar_event", { epgKey });
        if (seq !== modalCalendarSeq) return;
        trackedEvent = null;
        if (epgKey) calendarEventKeySet.delete(epgKey);
        trackedEventsCount = calendarEventKeySet.size;
        renderTrackedEventsCount();
        applyButtonMode();
        renderEPG();
      } catch (e) {
        if (seq !== modalCalendarSeq) return;
        const msg = formatGoogleError(e);
        if (String(e).includes("GOOGLE_SESSION_EXPIRED")) {
          authStatus.loggedIn = false;
          authStatus.profile = null;
          renderAccountSection();
        }
        applyButtonMode();
        await showAlert(
          t("alert_error_title"),
          t("google_calendar_remove_error", { error: msg }),
        );
      }
      return;
    }

    // Add new calendar event
    const startDatetime = dateToLocalIsoWithOffset(airing.startDate);
    const endDatetime = dateToLocalIsoWithOffset(airing.stopDate);
    if (!startDatetime || !endDatetime) return;

    btn.disabled = true;
    if (btnText) btnText.textContent = t("google_saving");

    try {
      const summary = buildCalendarSummary(title, tmdbData);
      const tmdbYear = tmdbData?.releaseDate
        ? String(tmdbData.releaseDate).slice(0, 4)
        : null;

      const result = await invoke("google_add_calendar_event", {
        req: {
          epgKey,
          sourceId,
          channelId,
          startTs,
          stopTs,
          summary,
          channelName: airing.channelName || "",
          sourceName: airing.sourceName || "",
          programmeDesc: desc || null,
          tmdbRating:
            typeof tmdbData?.voteAverage === "number"
              ? tmdbData.voteAverage
              : null,
          tmdbYear,
          startDatetime,
          endDatetime,
          timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
          reminderMinutes: 10,
          calendarId: null,
        },
      });

      if (seq !== modalCalendarSeq) return;

      trackedEvent = {
        epgKey,
        eventId: result.eventId,
        htmlLink: result.htmlLink,
        calendarId: result.calendarId,
        calendarName: result.calendarName,
      };
      if (epgKey) calendarEventKeySet.add(epgKey);
      trackedEventsCount = calendarEventKeySet.size;
      renderTrackedEventsCount();
      applyButtonMode();
      renderEPG();
    } catch (e) {
      if (seq !== modalCalendarSeq) return;
      const msg = formatGoogleError(e);
      if (String(e).includes("GOOGLE_SESSION_EXPIRED")) {
        authStatus.loggedIn = false;
        authStatus.profile = null;
        renderAccountSection();
      }
      applyButtonMode();
      await showAlert(
        t("alert_error_title"),
        t("google_calendar_error", { error: msg }),
      );
    }
  };
}
