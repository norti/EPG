import { state, invoke, setFavoritesList } from "./state.js";
import { t } from "./i18n.js";
import { TraktDates } from "../trakt/trakt-dates.js";
import { iconHtml, escapeHtml } from "./ui.js";
import { formatDateLabel, renderEPG } from "./epg.js";
import { updateModalFavStar } from "./modal.js";

export async function toggleFavorite(title) {
  const isFav = await invoke("toggle_favorite", { title });
  if (isFav) {
    if (!state.favoritesSet.has(title)) {
      state.favorites.push(title);
      state.favoritesSet.add(title);
    }
  } else {
    setFavoritesList(state.favorites.filter((item) => item !== title));
  }
  updateModalFavStar();
  renderEPG();
  renderFavoritesList();
  return isFav;
}

export function renderFavoritesList() {
  const content = document.getElementById("favListContent");
  const activeDayEl = document.getElementById("favActiveDay");
  content.innerHTML = "";
  if (activeDayEl) {
    activeDayEl.classList.add("u-hidden");
    activeDayEl.textContent = "";
  }

  if (state.favorites.length === 0) {
    content.innerHTML = `<p class="empty-list-message">${t("no_favorites")}</p>`;
    return;
  }

  // Single O(P) scan using O(1) Set lookup instead of O(F * P)
  const matchedList = [];
  const matchedTitles = new Set();
  for (const p of state.epgData.programmes) {
    if (state.favoritesSet.has(p.title)) {
      matchedTitles.add(p.title);
      matchedList.push(p);
    }
  }

  matchedList.sort((a, b) => a.start - b.start);

  const groupedByDay = {};
  matchedList.forEach((prog) => {
    const dayKey = formatDateLabel(
      `${prog.start.getFullYear()}-${String(prog.start.getMonth() + 1).padStart(2, "0")}-${String(prog.start.getDate()).padStart(2, "0")}`,
    );
    if (!groupedByDay[dayKey]) groupedByDay[dayKey] = [];
    groupedByDay[dayKey].push(prog);
  });

  const unmatchedFavorites = state.favorites.filter(
    (favTitle) => !matchedTitles.has(favTitle),
  );

  const frag = document.createDocumentFragment();

  if (Object.keys(groupedByDay).length === 0) {
    state.favorites.forEach((favTitle) => {
      const row = document.createElement("div");
      row.className = "fav-item-row";
      row.innerHTML = `
                        <span class="fav-item-title">${escapeHtml(favTitle)}</span>
                        <button class="btn btn-red btn-icon-only fav-remove-btn">${iconHtml("trash", 14)}</button>
                    `;
      row.querySelector("button").onclick = () => toggleFavorite(favTitle);
      frag.appendChild(row);
    });
    content.appendChild(frag);
    return;
  }

  const channelNameMap = new Map(
    (state.epgData.channels || []).map((c) => [c.id, c.name]),
  );

  for (const [dayHeader, progs] of Object.entries(groupedByDay)) {
    const groupDiv = document.createElement("div");
    groupDiv.className = "fav-day-group";
    groupDiv.dataset.dayHeading = dayHeader;

    const header = document.createElement("div");
    header.className = "fav-day-header";
    header.textContent = dayHeader;
    groupDiv.appendChild(header);

    progs.forEach((prog) => {
      const chName = channelNameMap.get(prog.channel) || prog.channel;
      const startStr = prog.start.toTimeString().substring(0, 5);

      const row = document.createElement("div");
      row.className = "fav-item-row";
      row.innerHTML = `
                        <span class="start-badge">${escapeHtml(startStr)}</span>
                        <span class="fav-item-title">${escapeHtml(prog.title)}</span>
                        <span class="channel-badge">${escapeHtml(chName)}</span>
                        <button class="btn btn-red btn-icon-only fav-remove-btn">${iconHtml("trash", 14)}</button>
                    `;
      row.querySelector("button").onclick = () => toggleFavorite(prog.title);
      groupDiv.appendChild(row);
    });

    frag.appendChild(groupDiv);
  }

  if (unmatchedFavorites.length > 0) {
    const unscheduledHeading = t("favorites_unscheduled");
    const groupDiv = document.createElement("div");
    groupDiv.className = "fav-day-group";
    groupDiv.dataset.dayHeading = unscheduledHeading;

    const header = document.createElement("div");
    header.className = "fav-day-header";
    header.textContent = unscheduledHeading;
    groupDiv.appendChild(header);

    unmatchedFavorites.forEach((favTitle) => {
      const row = document.createElement("div");
      row.className = "fav-item-row";
      row.innerHTML = `
                        <span class="fav-item-title">${escapeHtml(favTitle)}</span>
                        <button class="btn btn-red btn-icon-only fav-remove-btn">${iconHtml("trash", 14)}</button>
                    `;
      row.querySelector("button").onclick = () => toggleFavorite(favTitle);
      groupDiv.appendChild(row);
    });

    frag.appendChild(groupDiv);
  }

  content.appendChild(frag);

  if (activeDayEl && typeof TraktDates.attachStickyDayHeader === "function") {
    TraktDates.attachStickyDayHeader(content, activeDayEl, {
      groupSelector: ".fav-day-group",
    });
  }
}
