import { state, invoke } from "./state.js";
import { t } from "./i18n.js";
import { TraktDates } from "../trakt/trakt-dates.js";
import { iconHtml, escapeHtml } from "./ui.js";
import { formatDateLabel, renderEPG } from "./epg.js";
import { updateModalFavStar } from "./modal.js";

export async function toggleFavorite(title) {
  const isFav = await invoke("toggle_favorite", { title });
  if (isFav) {
    if (!state.favorites.includes(title)) state.favorites.push(title);
  } else {
    state.favorites = state.favorites.filter((item) => item !== title);
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

  let matchedList = [];
  state.favorites.forEach((favTitle) => {
    const progs = state.epgData.programmes.filter((p) => p.title === favTitle);
    progs.forEach((p) => matchedList.push(p));
  });

  matchedList.sort((a, b) => a.start - b.start);

  const groupedByDay = {};
  matchedList.forEach((prog) => {
    const dayKey = formatDateLabel(
      `${prog.start.getFullYear()}-${String(prog.start.getMonth() + 1).padStart(2, "0")}-${String(prog.start.getDate()).padStart(2, "0")}`,
    );
    if (!groupedByDay[dayKey]) groupedByDay[dayKey] = [];
    groupedByDay[dayKey].push(prog);
  });

  if (Object.keys(groupedByDay).length === 0) {
    state.favorites.forEach((favTitle) => {
      const row = document.createElement("div");
      row.className = "fav-item-row";
      row.innerHTML = `
                        <span class="fav-item-title">${escapeHtml(favTitle)}</span>
                        <button class="btn btn-red btn-icon-only fav-remove-btn">${iconHtml("trash", 14)}</button>
                    `;
      row.querySelector("button").onclick = () => toggleFavorite(favTitle);
      content.appendChild(row);
    });
    return;
  }

  for (const [dayHeader, progs] of Object.entries(groupedByDay)) {
    const groupDiv = document.createElement("div");
    groupDiv.className = "fav-day-group";
    groupDiv.dataset.dayHeading = dayHeader;

    const header = document.createElement("div");
    header.className = "fav-day-header";
    header.textContent = dayHeader;
    groupDiv.appendChild(header);

    progs.forEach((prog) => {
      const ch = state.epgData.channels.find((c) => c.id === prog.channel);
      const chName = ch ? ch.name : prog.channel;
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

    content.appendChild(groupDiv);
  }

  if (activeDayEl && typeof TraktDates.attachStickyDayHeader === "function") {
    TraktDates.attachStickyDayHeader(content, activeDayEl, {
      groupSelector: ".fav-day-group",
    });
  }
}
