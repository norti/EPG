import { state, invoke, LOCALE_COMPARE_OPTIONS } from "./state.js";
import { t, getCurrentLang } from "./i18n.js";
import { Countries } from "../utils/countries.js";
import {
  iconHtml,
  showConfirm,
  showAlert,
  updateAllFavBtnState,
  renderEmptySourcesState,
  escapeHtml,
} from "./ui.js";
import { fetchEPG, renderEPG, invalidateEpgSourceCache } from "./epg.js";
import {
  loadChannelOrderSourcesDropdown,
  renderChannelOrderList,
  invalidateChannelOrderCache,
} from "./channels.js";

export async function refreshSourcesFromDb() {
  invalidateChannelOrderCache();
  state.sources = await invoke("get_sources");
  loadSourcesDropdown();
  loadChannelOrderSourcesDropdown();
}

async function saveSourceOrderFromDom() {
  const sourcesList = document.getElementById("sourcesList");
  const sourceSelect = document.getElementById("sourceSelect");
  if (!sourcesList || !sourceSelect) return;

  const items = Array.from(sourcesList.querySelectorAll(".source-item"));
  const newOrder = items.map((el) => el.dataset.sourceId);
  // Update the in-memory sources array to match new order
  newOrder.forEach((id, idx) => {
    const src = state.sources.find((s) => s.id === id);
    if (src) src._sortPos = idx;
  });
  state.sources.sort((a, b) => (a._sortPos ?? 0) - (b._sortPos ?? 0));
  // Reload dropdown option order
  Array.from(sourceSelect.options).forEach((opt) => {
    if (opt.value !== "__ALL_FAVORITES__") opt.remove();
  });
  state.sources.forEach((s) => {
    const opt = document.createElement("option");
    opt.value = s.id;
    opt.textContent = `${s.name} [${s.countryCode || "HU"}]`;
    sourceSelect.appendChild(opt);
  });
  sourceSelect.value = state.currentSourceId;
  // Renumber order-index badges
  items.forEach((el, i) => {
    const badge = el.querySelector(".source-order-index");
    if (badge) badge.textContent = i + 1;
  });
  await invoke("save_source_order", { order: newOrder });
  invalidateEpgSourceCache("__ALL_FAVORITES__");
  if (state.currentSourceId === "__ALL_FAVORITES__") {
    state.settingsEpgDirty = true;
  }
}

export function loadSourcesDropdown() {
  const sourceSelect = document.getElementById("sourceSelect");
  const sourcesList = document.getElementById("sourcesList");
  sourceSelect.innerHTML = "";
  sourcesList.innerHTML = "";

  if (state.sources.length > 0) {
    const allFavOpt = document.createElement("option");
    allFavOpt.value = "__ALL_FAVORITES__";
    allFavOpt.textContent = t("all_favorites_source");
    sourceSelect.appendChild(allFavOpt);
  }

  state.sources.forEach((s, idx) => {
    const opt = document.createElement("option");
    opt.value = s.id;
    opt.textContent = `${s.name} [${s.countryCode || "HU"}]`;
    sourceSelect.appendChild(opt);

    const isDownloading = state.downloadingSources.has(s.id);
    const lastDateHtml = isDownloading
      ? `<span class="source-updating-status">${escapeHtml(t("status_updating"))}</span>`
      : escapeHtml(
          s.lastDownloaded
            ? new Date(s.lastDownloaded).toLocaleString(
                getCurrentLang() === "english" ? "en-US" : "hu-HU",
              )
            : t("never_updated"),
        );

    const isUpdatingAll = state.isUpdatingAllSources;
    const allCount =
      s.channelCount !== undefined && s.channelCount !== null
        ? s.channelCount
        : 0;
    const favCount =
      s.favoriteCount !== undefined && s.favoriteCount !== null
        ? s.favoriteCount
        : 0;
    const hidCount =
      s.hiddenCount !== undefined && s.hiddenCount !== null ? s.hiddenCount : 0;

    const item = document.createElement("div");
    item.className = "source-item";
    item.dataset.sourceId = s.id;
    item.innerHTML = `
                    <div class="source-row-header">
                        <div class="source-title-group">
                            <span class="source-drag-handle" title="${escapeHtml(t("sources_drag_hint"))}">${iconHtml("grip-vertical", 16)}</span>
                            <span class="source-order-index">${idx + 1}</span>
                            <strong class="source-name">${escapeHtml(s.name)} <span class="source-country-badge">${escapeHtml(s.countryCode || "HU")}</span> <span class="source-offset-badge">(Offset: ${s.timeOffset >= 0 ? "+" : ""}${s.timeOffset || 0}h)</span></strong>
                        </div>
                        <div class="source-header-actions">
                            <div class="source-reorder-btns">
                                <button class="btn-order-arrow btn-up" title="${escapeHtml(t("btn_move_up"))}">${iconHtml("caret-up-filled", 13)}</button>
                                <button class="btn-order-arrow btn-down" title="${escapeHtml(t("btn_move_down"))}">${iconHtml("caret-down-filled", 13)}</button>
                            </div>
                            <div class="source-actions">
                                <button class="btn btn-green btn-source-update"${isDownloading || isUpdatingAll ? " disabled" : ""}>${escapeHtml(isDownloading ? t("status_updating") : t("btn_update"))}</button>
                                <button class="btn btn-blue btn-source-edit"${isDownloading || isUpdatingAll ? " disabled" : ""}>${escapeHtml(t("btn_edit"))}</button>
                                <button class="btn btn-red btn-source-delete"${isDownloading || isUpdatingAll ? " disabled" : ""}>${escapeHtml(t("btn_delete"))}</button>
                            </div>
                        </div>
                    </div>
                    <div class="source-url" title="${escapeHtml(s.url)}">${escapeHtml(s.url)}</div>
                    <div class="source-update-info">${escapeHtml(t("last_updated"))} ${lastDateHtml}</div>
                    <div class="source-stats-row">
                        <span class="source-stat-badge source-stat-all">${escapeHtml(t("source_stat_all"))}: ${allCount}</span>
                        <span class="source-stat-badge source-stat-fav">${escapeHtml(t("source_stat_fav"))}: ${favCount}</span>
                        <span class="source-stat-badge source-stat-hid">${escapeHtml(t("source_stat_hid"))}: ${hidCount}</span>
                    </div>
                `;

    const updateBtn = item.querySelector(".btn-source-update");
    if (updateBtn) updateBtn.onclick = () => downloadSource(s.id);

    const editBtn = item.querySelector(".btn-source-edit");
    if (editBtn) editBtn.onclick = () => editSource(s.id);

    const deleteBtn = item.querySelector(".btn-source-delete");
    if (deleteBtn) deleteBtn.onclick = () => deleteSource(s.id);

    // ▲ Up arrow
    item.querySelector(".btn-up").onclick = async () => {
      const prev = item.previousElementSibling;
      if (prev && prev.classList.contains("source-item")) {
        sourcesList.insertBefore(item, prev);
        await saveSourceOrderFromDom();
      }
    };

    // ▼ Down arrow
    item.querySelector(".btn-down").onclick = async () => {
      const next = item.nextElementSibling;
      if (next && next.classList.contains("source-item")) {
        sourcesList.insertBefore(next, item);
        await saveSourceOrderFromDom();
      }
    };

    // Drag-and-drop (reuse channel order drag pattern)
    const handle = item.querySelector(".source-drag-handle");
    handle.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return;
      item.classList.add("dragging");
      try {
        handle.setPointerCapture(e.pointerId);
      } catch (_) {}

      const onPointerMove = (moveEvent) => {
        const targetEl = document
          .elementFromPoint(moveEvent.clientX, moveEvent.clientY)
          ?.closest(".source-item");
        if (
          targetEl &&
          targetEl !== item &&
          targetEl.parentElement === sourcesList
        ) {
          const rect = targetEl.getBoundingClientRect();
          if (moveEvent.clientY < rect.top + rect.height / 2) {
            sourcesList.insertBefore(item, targetEl);
          } else {
            sourcesList.insertBefore(item, targetEl.nextSibling);
          }
        }
      };

      const onPointerUp = async (upEvent) => {
        document.removeEventListener("pointermove", onPointerMove);
        document.removeEventListener("pointerup", onPointerUp);
        document.removeEventListener("pointercancel", onPointerUp);
        try {
          handle.releasePointerCapture(upEvent.pointerId);
        } catch (_) {}
        item.classList.remove("dragging");
        await saveSourceOrderFromDom();
      };

      document.addEventListener("pointermove", onPointerMove);
      document.addEventListener("pointerup", onPointerUp);
      document.addEventListener("pointercancel", onPointerUp);
    });

    sourcesList.appendChild(item);
  });

  if (!state.currentSourceId && state.sources.length > 0) {
    state.currentSourceId = "__ALL_FAVORITES__";
  }
  sourceSelect.value = state.currentSourceId;
  updateAllFavBtnState();

  sourceSelect.onchange = async (e) => {
    state.activeSingleChannelId = null;
    state.currentSourceId = e.target.value;
    updateAllFavBtnState();
    await invoke("set_setting", {
      key: "current_source_id",
      value: state.currentSourceId,
    });
    fetchEPG();
  };

  const allFavBtn = document.getElementById("allFavChannelsBtn");
  if (allFavBtn) {
    allFavBtn.onclick = async () => {
      state.activeSingleChannelId = null;
      if (state.currentSourceId === "__ALL_FAVORITES__") {
        if (state.sources.length > 0) {
          state.currentSourceId = state.sources[0].id;
        }
      } else {
        state.currentSourceId = "__ALL_FAVORITES__";
      }
      sourceSelect.value = state.currentSourceId;
      updateAllFavBtnState();
      await invoke("set_setting", {
        key: "current_source_id",
        value: state.currentSourceId,
      });
      fetchEPG();
    };
  }

  // A-Z sort button
  const sortAzBtn = document.getElementById("sortSourcesAzBtn");
  if (sortAzBtn) {
    sortAzBtn.onclick = async () => {
      const items = Array.from(sourcesList.querySelectorAll(".source-item"));
      if (items.length === 0) return;
      items.sort((a, b) => {
        const srcA =
          state.sources.find((s) => s.id === a.dataset.sourceId)?.name || "";
        const srcB =
          state.sources.find((s) => s.id === b.dataset.sourceId)?.name || "";
        return srcA.localeCompare(
          srcB,
          getCurrentLang() === "hungarian" ? "hu" : "en",
          LOCALE_COMPARE_OPTIONS,
        );
      });
      items.forEach((el) => {
        sourcesList.appendChild(el);
      });
      await saveSourceOrderFromDom();
    };
  }
}

export async function downloadSource(id, options = {}) {
  const src = state.sources.find((s) => s.id === id);
  if (!src || state.downloadingSources.has(id)) return false;

  state.downloadingSources.add(id);
  loadSourcesDropdown();

  document.getElementById("status").textContent = t("status_downloading", {
    name: src.name,
  });
  try {
    await invoke("update_source_epg", { sourceId: src.id, url: src.url });
    state.downloadingSources.delete(id);
    invalidateEpgSourceCache(id);
    await refreshSourcesFromDb();

    document.getElementById("status").textContent = t("status_download_success");
    if (
      !options.skipFetch &&
      (id === state.currentSourceId ||
        state.currentSourceId === "__ALL_FAVORITES__")
    ) {
      await fetchEPG({ forceReload: true });
    }
    return true;
  } catch (err) {
    state.downloadingSources.delete(id);
    await refreshSourcesFromDb();
    console.error(err);
    await showAlert(
      t("alert_error_title"),
      t("status_download_error") + ": " + err,
    );
    document.getElementById("status").textContent = t("status_download_error");
    return false;
  }
}

export async function updateAllSources() {
  if (state.isUpdatingAllSources) return;
  if (!state.sources || state.sources.length === 0) {
    await showAlert(t("alert_warning_title"), t("no_sources_to_update"));
    return;
  }

  state.isUpdatingAllSources = true;
  const btn = document.getElementById("updateAllSourcesBtn");
  const originalText = btn ? btn.textContent : t("btn_update_all");

  loadSourcesDropdown();

  let updatedCount = 0;
  let hasErrors = false;

  try {
    for (let i = 0; i < state.sources.length; i++) {
      const s = state.sources[i];
      const progressLabel = `${t("btn_update_all_progress")} (${i + 1}/${state.sources.length}): ${s.name}...`;
      if (btn) btn.textContent = progressLabel;
      const statusEl = document.getElementById("status");
      if (statusEl) {
        statusEl.textContent = t("status_downloading", {
          name: s.name,
        });
      }

      state.downloadingSources.add(s.id);
      loadSourcesDropdown();

      try {
        await invoke("update_source_epg", { sourceId: s.id, url: s.url });
        updatedCount++;
      } catch (err) {
        console.error(`Hiba a(z) ${s.name} forrás frissítésekor:`, err);
        hasErrors = true;
      } finally {
        state.downloadingSources.delete(s.id);
      }
    }
  } finally {
    state.isUpdatingAllSources = false;
  }

  await refreshSourcesFromDb();
  invalidateEpgSourceCache();

  if (btn) btn.textContent = originalText;
  const statusEl = document.getElementById("status");
  if (hasErrors) {
    if (statusEl) statusEl.textContent = t("status_download_error");
    await showAlert(
      t("alert_error_title"),
      t("status_update_all_done_errors", {
        count: updatedCount,
        total: state.sources.length,
      }),
    );
  } else {
    if (statusEl) statusEl.textContent = t("status_download_success");
  }

  if (state.currentSourceId) {
    await fetchEPG({ forceReload: true });
  }
}

function resetSourceForm() {
  document.getElementById("editSourceId").value = "";
  document.getElementById("sourceNameInput").value = "";
  document.getElementById("sourceUrlInput").value = "";
  document.getElementById("sourceCountrySelect").value = "HU";
  document.getElementById("sourceOffsetInput").value = 0;
  document.getElementById("saveSourceBtn").textContent = t("btn_save_source");
  const cancelBtn = document.getElementById("cancelEditSourceBtn");
  if (cancelBtn) cancelBtn.classList.add("u-hidden");
}

export function editSource(id) {
  const src = state.sources.find((s) => s.id === id);
  if (!src) return;

  document.getElementById("editSourceId").value = src.id;
  document.getElementById("sourceNameInput").value = src.name;
  document.getElementById("sourceUrlInput").value = src.url;
  const normalizedCountry = Countries.normalizeCode(src.countryCode);
  document.getElementById("sourceCountrySelect").value = normalizedCountry;
  document.getElementById("sourceOffsetInput").value = src.timeOffset || 0;
  document.getElementById("saveSourceBtn").textContent = t("btn_save_source");
  const cancelBtn = document.getElementById("cancelEditSourceBtn");
  if (cancelBtn) cancelBtn.classList.remove("u-hidden");
}

export async function deleteSource(id) {
  const confirmed = await showConfirm(
    t("confirm_delete_source_title"),
    t("confirm_delete_source"),
  );
  if (!confirmed) return;

  try {
    await invoke("delete_source", { sourceId: id });
    invalidateEpgSourceCache(id);
    if (document.getElementById("editSourceId")?.value === id) {
      resetSourceForm();
    }
    await refreshSourcesFromDb();

    if (state.sources.length === 0) {
      state.currentSourceId = "";
      await invoke("set_setting", {
        key: "current_source_id",
        value: "",
      });
      updateAllFavBtnState();
    } else if (state.currentSourceId === id) {
      state.currentSourceId = "__ALL_FAVORITES__";
      await invoke("set_setting", {
        key: "current_source_id",
        value: state.currentSourceId,
      });
    }

    loadSourcesDropdown();
    if (state.currentSourceId) {
      await fetchEPG();
    } else {
      state.epgData = { channels: [], programmes: [] };
      renderEmptySourcesState();
    }
  } catch (err) {
    console.error(err);
    await showAlert(t("alert_error_title"), err);
  }
}

export function initSources() {
  const updateAllBtn = document.getElementById("updateAllSourcesBtn");
  if (updateAllBtn) {
    updateAllBtn.onclick = () => updateAllSources();
  }

  document.getElementById("clearDbBtn").onclick = async () => {
    const confirmed = await showConfirm(
      t("confirm_clear_db_title"),
      t("confirm_clear_db"),
    );
    if (!confirmed) return;

    try {
      state.activeSingleChannelId = null;
      await invoke("clear_entire_database");
      invalidateEpgSourceCache();
      await refreshSourcesFromDb();
      state.epgData = { channels: [], programmes: [] };
      renderEPG();
      renderChannelOrderList();
      await showAlert(t("alert_success_title"), t("alert_db_cleared"));
    } catch (err) {
      console.error(err);
      await showAlert(t("alert_error_title"), err);
    }
  };

  const cancelEditBtn = document.getElementById("cancelEditSourceBtn");
  if (cancelEditBtn) {
    cancelEditBtn.onclick = () => resetSourceForm();
  }

  document.getElementById("saveSourceBtn").onclick = async () => {
    const editId = document.getElementById("editSourceId").value;
    const name = document.getElementById("sourceNameInput").value.trim();
    const url = document.getElementById("sourceUrlInput").value.trim();
    const countryCode = document.getElementById("sourceCountrySelect").value;
    const timeOffset =
      parseInt(document.getElementById("sourceOffsetInput").value, 10) || 0;

    if (!name || !url)
      return await showAlert(t("alert_warning_title"), t("alert_fill_fields"));

    const sourceObj = {
      id: editId ? editId : "src_" + Date.now(),
      name,
      url,
      lastDownloaded: null,
      countryCode,
      timeOffset,
    };

    await invoke("save_source", { source: sourceObj });
    invalidateEpgSourceCache(sourceObj.id);
    await refreshSourcesFromDb();

    resetSourceForm();

    if (!state.currentSourceId || editId === state.currentSourceId) {
      state.currentSourceId = sourceObj.id;
      document.getElementById("sourceSelect").value = state.currentSourceId;
      await fetchEPG({ forceReload: true });
    }
  };
}
