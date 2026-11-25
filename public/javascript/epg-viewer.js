async function init() {
  if (window.Countries) {
    window.Countries.populateSelect("sourceCountrySelect", {
      format: "source",
      selected: "HU",
    });
  }
  if (window.Icons) {
    window.Icons.replace();
  }
  try {
    settings = await invoke("get_settings");
  } catch (e) {
    console.error("Nem sikerült lekérni a beállításokat:", e);
    settings = {};
  }

  const initialLang = settings.language || "hungarian";
  await setLanguage(initialLang);

  if (window.TraktUI) {
    await window.TraktUI.init();
  }

  if (settings.view_step_min) currentStepMin = parseInt(settings.view_step_min);
  if (settings.view_minute_height)
    currentMinuteHeight = parseFloat(settings.view_minute_height);
  if (
    settings.view_layout === "horizontal" ||
    settings.view_layout === "vertical"
  )
    currentLayout = settings.view_layout;

  updateLayoutButtonUi();

  const viewSelect = document.getElementById("viewSelect");
  if (viewSelect) {
    viewSelect.value = String(currentStepMin);
  }

  const prevDayBtn = document.getElementById("prevDayBtn");
  if (prevDayBtn) prevDayBtn.onclick = () => navigateDate(-1);
  const nextDayBtn = document.getElementById("nextDayBtn");
  if (nextDayBtn) nextDayBtn.onclick = () => navigateDate(1);

  try {
    await refreshSourcesFromDb();
  } catch (err) {
    console.error("Failed to load sources during init:", err);
    sources = [];
  }
  try {
    favorites = await invoke("get_favorites");
  } catch (err) {
    console.error("Failed to load favorites during init:", err);
    favorites = [];
  }

  if (sources.length > 0) {
    if (
      settings.current_source_id &&
      (settings.current_source_id === "__ALL_FAVORITES__" ||
        sources.some((s) => s.id === settings.current_source_id))
    ) {
      currentSourceId = settings.current_source_id;
    } else {
      currentSourceId = "__ALL_FAVORITES__";
    }
    document.getElementById("sourceSelect").value = currentSourceId;
    updateAllFavBtnState();
    await fetchEPG();
  } else {
    currentSourceId = "";
    updateAllFavBtnState();
    document.getElementById("status").textContent = t("status_ready");
    renderEmptySourcesState();
  }
}

window.addEventListener("DOMContentLoaded", init);
