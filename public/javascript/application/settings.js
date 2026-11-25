import { state, invoke, setFavoritesList } from "./state.js";
import { t, setLanguage, restoreLanguageFiles } from "./i18n.js";
import {
  showConfirm,
  showAlert,
  formatByteSize,
  setExportImportStatus,
} from "./ui.js";
import { refreshSourcesFromDb } from "./sources.js";
import { renderFavoritesList } from "./favorites.js";
import { fetchEPG, invalidateEpgSourceCache } from "./epg.js";
import { loadTmdbSettings, clearTmdbPosterMemoryCache } from "../tmdb/tmdb.js";
import {
  loadChannelLogosTab,
  loadChannelSortPresetsDropdown,
  loadChannelOrderSourcesDropdown,
  renderChannelOrderList,
  clearChannelLogoPreviewCache,
  invalidateChannelOrderCache,
} from "./channels.js";
import { loadSettingsTab as loadTraktSettingsTab } from "../trakt/trakt-ui.js";
import { clearTraktImageCaches } from "../trakt/trakt-dashboard.js";

export function switchSettingsTab(tabName) {
  document.querySelectorAll(".settings-nav-item").forEach((btn) => {
    if (btn.dataset.tab === tabName) btn.classList.add("active");
    else btn.classList.remove("active");
  });

  document
    .querySelectorAll(".settings-tab-pane")
    .forEach((pane) => pane.classList.remove("active"));
  if (tabName === "sources") {
    document.getElementById("tabPaneSources").classList.add("active");
  } else if (tabName === "favorites") {
    document.getElementById("tabPaneFavorites").classList.add("active");
    renderFavoritesList();
  } else if (tabName === "channels") {
    document.getElementById("tabPaneChannels").classList.add("active");
    invalidateChannelOrderCache();
    loadChannelOrderSourcesDropdown();
    loadChannelSortPresetsDropdown();
    renderChannelOrderList();
  } else if (tabName === "logos") {
    document.getElementById("tabPaneLogos").classList.add("active");
    loadChannelLogosTab();
  } else if (tabName === "tmdb") {
    document.getElementById("tabPaneTmdb").classList.add("active");
    loadTmdbSettings();
  } else if (tabName === "trakt") {
    document.getElementById("tabPaneTrakt").classList.add("active");
    loadTraktSettingsTab();
  } else if (tabName === "export") {
    document.getElementById("tabPaneExport").classList.add("active");
    updateCacheSectionStats();
    updateDatabaseStats();
  }
}

function initSettingsExportHandlers() {
  document.querySelectorAll(".settings-nav-item").forEach((item) => {
    item.onclick = () => switchSettingsTab(item.dataset.tab);
  });

  document.getElementById("exportSettingsBtn").onclick = async () => {
    try {
      setExportImportStatus(
        "settingsDataStatus",
        t("status_exporting"),
        "loading",
      );
      const backup = await invoke("export_settings");
      const json = JSON.stringify(backup, null, 2);
      const blob = new Blob([json], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `epg-viewer-settings-${new Date().toISOString().slice(0, 10)}.json`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
      setExportImportStatus(
        "settingsDataStatus",
        t("alert_export_success"),
        "success",
      );
    } catch (err) {
      setExportImportStatus(
        "settingsDataStatus",
        t("alert_export_error"),
        "error",
      );
      await showAlert(
        t("alert_error_title"),
        t("alert_export_error") + ": " + err,
      );
    }
  };

  document.getElementById("importSettingsInput").onchange = async (e) => {
    const file = e.target.files[0];
    if (!file) return;
    try {
      setExportImportStatus(
        "settingsDataStatus",
        t("status_importing"),
        "loading",
      );
      const text = await file.text();
      const backup = JSON.parse(text);
      await invoke("import_settings", { backup });
      setExportImportStatus(
        "settingsDataStatus",
        t("alert_import_success"),
        "success",
      );
      await showAlert(t("alert_success_title"), t("alert_import_success"));
      invalidateChannelOrderCache();
      invalidateEpgSourceCache();
      await refreshSourcesFromDb();
      setFavoritesList(await invoke("get_favorites"));
      await fetchEPG({ forceReload: true });
      renderFavoritesList();
      await renderChannelOrderList();
    } catch (err) {
      setExportImportStatus(
        "settingsDataStatus",
        t("alert_import_error"),
        "error",
      );
      await showAlert(
        t("alert_error_title"),
        t("alert_import_error") + ": " + err,
      );
    }
    e.target.value = "";
  };
}

function parseCsvLine(line, delimiter = ",") {
  const result = [];
  let cur = "";
  let inQuotes = false;
  for (let i = 0; i < line.length; i++) {
    const char = line[i];
    if (char === '"') {
      if (inQuotes && line[i + 1] === '"') {
        cur += '"';
        i++;
      } else {
        inQuotes = !inQuotes;
      }
    } else if (char === delimiter && !inQuotes) {
      result.push(cur);
      cur = "";
    } else {
      cur += char;
    }
  }
  result.push(cur);
  return result;
}

function parseCsvRows(text) {
  // Normalize CRLF to LF
  const clean = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  const rawLines = clean.split("\n");
  const lines = [];
  let acc = "";
  let inQuotes = false;

  for (const rawLine of rawLines) {
    if (!inQuotes) {
      acc = rawLine;
    } else {
      acc += "\n" + rawLine;
    }
    // Count quotes in rawLine
    let quoteCount = 0;
    for (let i = 0; i < rawLine.length; i++) {
      if (rawLine[i] === '"') quoteCount++;
    }
    if (quoteCount % 2 === 1) {
      inQuotes = !inQuotes;
    }
    if (!inQuotes && acc.trim()) {
      lines.push(acc);
      acc = "";
    }
  }
  if (acc.trim()) {
    lines.push(acc);
  }
  if (lines.length === 0) return [];

  // Detect delimiter: comma or semicolon based on first line
  const firstLine = lines[0];
  const delimiter =
    (firstLine.match(/;/g) || []).length > (firstLine.match(/,/g) || []).length
      ? ";"
      : ",";

  const headers = parseCsvLine(lines[0], delimiter).map((h) =>
    h
      .trim()
      .replace(/^[\uFEFF]/, "")
      .toLowerCase(),
  );

  const rows = [];
  for (let i = 1; i < lines.length; i++) {
    const cols = parseCsvLine(lines[i], delimiter);
    const obj = {};
    headers.forEach((h, idx) => {
      obj[h] = cols[idx] !== undefined ? cols[idx].trim() : "";
    });
    rows.push(obj);
  }
  return rows;
}

function escapeCsvField(val) {
  if (val === null || val === undefined) return "";
  const str = String(val);
  if (
    str.includes(",") ||
    str.includes('"') ||
    str.includes("\n") ||
    str.includes("\r") ||
    str.includes(";")
  ) {
    return `"${str.replace(/"/g, '""')}"`;
  }
  return str;
}

function initChannelExportImportHandlers() {
  document.getElementById("exportChannelDataBtn").onclick = async () => {
  try {
    setExportImportStatus(
      "channelDataStatus",
      t("status_exporting"),
      "loading",
    );
    const exportData = await invoke("export_channel_data");
    const channels = exportData.channels || [];

    const headers = [
      "source_id",
      "source_name",
      "source_url",
      "channel_id",
      "channel_name",
      "favorite",
      "hidden",
      "channel_position",
      "custom_icon_url",
      "custom_name",
    ];

    const csvRows = [headers.join(";")];
    for (const ch of channels) {
      const row = [
        escapeCsvField(ch.sourceId || ""),
        escapeCsvField(ch.sourceName || ""),
        escapeCsvField(ch.sourceUrl || ""),
        escapeCsvField(ch.channelId || ""),
        escapeCsvField(ch.channelName || ""),
        escapeCsvField(ch.isFavorite ? 1 : 0),
        escapeCsvField(ch.isHidden ? 1 : 0),
        escapeCsvField(
          ch.position !== null && ch.position !== undefined ? ch.position : "",
        ),
        escapeCsvField(ch.customIcon || ""),
        escapeCsvField(ch.customName || ""),
      ];
      csvRows.push(row.join(";"));
    }

    const csvString = "\uFEFF" + csvRows.join("\r\n");
    const blob = new Blob([csvString], {
      type: "text/csv;charset=utf-8;",
    });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `epg-channels-${new Date().toISOString().slice(0, 10)}.csv`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
    setExportImportStatus(
      "channelDataStatus",
      t("alert_export_success"),
      "success",
    );
  } catch (err) {
    setExportImportStatus(
      "channelDataStatus",
      t("alert_channel_export_error"),
      "error",
    );
    await showAlert(
      t("alert_error_title"),
      t("alert_channel_export_error") + ": " + err,
    );
  }
};

document.getElementById("importChannelDataInput").onchange = async (e) => {
  const file = e.target.files[0];
  if (!file) return;
  try {
    setExportImportStatus(
      "channelDataStatus",
      t("status_importing"),
      "loading",
    );
    const text = await file.text();
    let channels = [];

    // Support both CSV and legacy JSON fallback
    if (
      file.name.toLowerCase().endsWith(".json") ||
      text.trim().startsWith("{")
    ) {
      const parsed = JSON.parse(text);
      channels = parsed.channels || [];
    } else {
      const rows = parseCsvRows(text);
      channels = rows.map((r) => {
        const favVal = (
          r["favorite"] ||
          r["is_favorite"] ||
          r["isfavorite"] ||
          "0"
        ).trim();
        const hidVal = (
          r["hidden"] ||
          r["is_hidden"] ||
          r["ishidden"] ||
          "0"
        ).trim();
        const posVal = (r["channel_position"] || r["position"] || "").trim();

        const sourceId = r["source_id"] || r["sourceid"] || "";
        const channelId = r["channel_id"] || r["channelid"] || "";

        const customIcon =
          r["custom_icon_url"] || r["custom_icon"] || r["customicon"] || "";
        const customName = r["custom_name"] || r["customname"] || "";

        return {
          sourceId,
          sourceName: r["source_name"] || r["sourcename"] || "",
          sourceUrl: r["source_url"] || r["sourceurl"] || "",
          channelId,
          channelName: r["channel_name"] || r["channelname"] || "",
          isFavorite: favVal === "1" || favVal.toLowerCase() === "true" ? 1 : 0,
          isHidden: hidVal === "1" || hidVal.toLowerCase() === "true" ? 1 : 0,
          position:
            posVal !== "" && !isNaN(parseInt(posVal, 10))
              ? parseInt(posVal, 10)
              : null,
          customIcon: customIcon || null,
          customName: customName || null,
        };
      });
    }

    await invoke("import_channel_data", {
      data: {
        version: 1,
        exportedAt: new Date().toISOString(),
        channels,
      },
    });
    setExportImportStatus(
      "channelDataStatus",
      t("alert_channel_import_success"),
      "success",
    );
    await showAlert(
      t("alert_success_title"),
      t("alert_channel_import_success"),
    );
    invalidateChannelOrderCache();
    invalidateEpgSourceCache();
    await refreshSourcesFromDb();
    await fetchEPG({ forceReload: true });
    await renderChannelOrderList();
  } catch (err) {
    setExportImportStatus(
      "channelDataStatus",
      t("alert_channel_import_error"),
      "error",
    );
    await showAlert(
      t("alert_error_title"),
      t("alert_channel_import_error") + ": " + err,
    );
  }
  e.target.value = "";
};
}

// ============================================================================
// Cache Management (ZIP Export / Import, Clear Logos & Posters)
// ============================================================================

export async function updateCacheSectionStats() {
  const logosCountEl = document.getElementById("cacheStatLogosCount");
  const logosSizeEl = document.getElementById("cacheStatLogosSize");
  const postersCountEl = document.getElementById("cacheStatPostersCount");
  const postersSizeEl = document.getElementById("cacheStatPostersSize");
  const totalSizeEl = document.getElementById("cacheStatTotalSize");

  if (!logosCountEl) return;

  try {
    const stats = await invoke("get_image_cache_stats");
    logosCountEl.textContent = stats.logoCount;
    logosSizeEl.textContent = `(${formatByteSize(stats.logoBytes)})`;
    postersCountEl.textContent = stats.posterCount;
    postersSizeEl.textContent = `(${formatByteSize(stats.posterBytes)})`;
    totalSizeEl.textContent = formatByteSize(stats.totalBytes);
  } catch (err) {
    console.error("Hiba a gyorsítótár statisztikák lekérésekor:", err);
  }
}

function initCacheClearHandlers() {
document.getElementById("clearLogoCacheBtn").onclick = async () => {
  const confirmed = await showConfirm(
    t("btn_clear_logo_cache"),
    t("confirm_clear_logo_cache"),
  );
  if (!confirmed) return;

  try {
    setExportImportStatus("cacheDataStatus", t("status_deleting"), "loading");
    await invoke("clear_logo_cache");
    clearChannelLogoPreviewCache();
    invalidateChannelOrderCache();
    invalidateEpgSourceCache();
    setExportImportStatus("cacheDataStatus", t("alert_cache_cleared"), "success");
    await showAlert(t("alert_success_title"), t("alert_cache_cleared"));
    await updateCacheSectionStats();
    await fetchEPG({ forceReload: true });
  } catch (err) {
    setExportImportStatus("cacheDataStatus", String(err), "error");
    await showAlert(t("alert_error_title"), String(err));
  }
};

document.getElementById("clearPosterCacheBtn").onclick = async () => {
  const confirmed = await showConfirm(
    t("btn_clear_poster_cache"),
    t("confirm_clear_poster_cache"),
  );
  if (!confirmed) return;

  try {
    setExportImportStatus("cacheDataStatus", t("status_deleting"), "loading");
    await invoke("clear_poster_cache");
    clearTraktImageCaches();
    clearTmdbPosterMemoryCache();
    setExportImportStatus("cacheDataStatus", t("alert_cache_cleared"), "success");
    await showAlert(t("alert_success_title"), t("alert_cache_cleared"));
    await updateCacheSectionStats();
  } catch (err) {
    setExportImportStatus("cacheDataStatus", String(err), "error");
    await showAlert(t("alert_error_title"), String(err));
  }
};

document.getElementById("clearAllCacheBtn").onclick = async () => {
  const confirmed = await showConfirm(
    t("btn_clear_all_cache"),
    t("confirm_clear_all_cache"),
  );
  if (!confirmed) return;

  try {
    setExportImportStatus("cacheDataStatus", t("status_deleting"), "loading");
    await invoke("clear_all_image_cache");
    clearChannelLogoPreviewCache();
    clearTraktImageCaches();
    clearTmdbPosterMemoryCache();
    invalidateChannelOrderCache();
    invalidateEpgSourceCache();
    setExportImportStatus("cacheDataStatus", t("alert_cache_cleared"), "success");
    await showAlert(t("alert_success_title"), t("alert_cache_cleared"));
    await updateCacheSectionStats();
    await updateDatabaseStats();
    await fetchEPG({ forceReload: true });
  } catch (err) {
    setExportImportStatus("cacheDataStatus", String(err), "error");
    await showAlert(t("alert_error_title"), String(err));
  }
};
}

export async function updateDatabaseStats() {
  const epgEl = document.getElementById("dbStatEpgSize");
  const imgEl = document.getElementById("dbStatImagesSize");
  const recEl = document.getElementById("dbStatReclaimable");
  if (!epgEl) return;

  try {
    const stats = await invoke("get_database_stats");
    epgEl.textContent = formatByteSize(stats.epgDbBytes);
    imgEl.textContent = formatByteSize(stats.imagesDbBytes);
    const expiredTotal =
      (stats.expiredHttpCacheRows || 0) +
      (stats.expiredTmdbNegativeRows || 0) +
      (stats.orphanedImageRows || 0);
    recEl.textContent =
      expiredTotal > 0
        ? `${formatByteSize(stats.freelistBytes)} (+${expiredTotal})`
        : formatByteSize(stats.freelistBytes);
  } catch (err) {
    console.error("Hiba az adatbázis statisztikák lekérésekor:", err);
  }
}

export function closeSettingsModal() {
  const overlay = document.getElementById("settingsOverlay");
  if (overlay) overlay.classList.remove("active");
  if (state.settingsEpgDirty) {
    state.settingsEpgDirty = false;
    fetchEPG({ forceReload: true });
  }
}

export function initSettings() {
  initSettingsExportHandlers();
  initChannelExportImportHandlers();
  initCacheClearHandlers();

  const maintainDatabaseBtn = document.getElementById("maintainDatabaseBtn");
  if (maintainDatabaseBtn) {
    maintainDatabaseBtn.onclick = async () => {
      const confirmed = await showConfirm(
        t("db_maintenance_title"),
        t("confirm_maintain_database"),
      );
      if (!confirmed) return;

      maintainDatabaseBtn.disabled = true;
      try {
        setExportImportStatus(
          "dbMaintenanceStatus",
          t("db_maintenance_running"),
          "loading",
        );
        const res = await invoke("maintain_database");
        const prunedCount =
          (res.prunedHttpCache || 0) +
          (res.prunedTmdbNegatives || 0) +
          (res.prunedOrphanImages || 0);
        const msg = t("db_maintenance_done")
          .replace("{before}", formatByteSize(res.beforeBytes))
          .replace("{after}", formatByteSize(res.afterBytes))
          .replace("{freed}", formatByteSize(res.freedBytes))
          .replace("{pruned}", String(prunedCount));
        setExportImportStatus("dbMaintenanceStatus", msg, "success");
        await showAlert(t("alert_success_title"), msg);
        await updateDatabaseStats();
        await updateCacheSectionStats();
      } catch (err) {
        setExportImportStatus("dbMaintenanceStatus", String(err), "error");
        await showAlert(t("alert_error_title"), String(err));
      } finally {
        maintainDatabaseBtn.disabled = false;
      }
    };
  }

  document.getElementById("favListBtn").onclick = () => {
    switchSettingsTab("favorites");
    document.getElementById("settingsOverlay").classList.add("active");
  };

  document.getElementById("settingsBtn").onclick = () => {
    switchSettingsTab("sources");
    document.getElementById("settingsOverlay").classList.add("active");
  };

  document.getElementById("settingsClose").onclick = () => {
    closeSettingsModal();
  };

  document.getElementById("languageSelect").onchange = (e) => {
    setLanguage(e.target.value);
  };

  const restoreLanguagesBtn = document.getElementById("restoreLanguagesBtn");
  if (restoreLanguagesBtn) {
    restoreLanguagesBtn.onclick = async () => {
      restoreLanguagesBtn.disabled = true;
      const res = await restoreLanguageFiles();
      restoreLanguagesBtn.disabled = false;
      await showAlert(
        res.success ? t("alert_success_title") : t("alert_error_title"),
        res.message,
      );
    };
  }
}
