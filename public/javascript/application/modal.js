import { state, invoke } from "./state.js";
import { t } from "./i18n.js";
import { iconHtml } from "./ui.js";
import { toggleFavorite } from "./favorites.js";
import { closeSettingsModal } from "./settings.js";
import { closeUpdateModal } from "./updater.js";
import {
  hidePosterZoom,
  resetModalTmdbUi,
  renderTmdbDetailsInModal,
  getCurrentModalLang,
  closeFixOverlay,
} from "../tmdb/tmdb.js";
import { attachToModal, detachFromModal } from "../trakt/trakt-ui.js";

export function setModalContext(title, channel, desc) {
  state.currentModalTitle = title || "";
  state.currentModalDesc = desc || "";
  state.currentModalChannel = channel || null;
  state.currentModalFetchIndex++;
}

export async function openModal(title, time, desc, channel) {
  state.currentModalTitle = title;
  state.currentModalDesc = desc || "";
  state.currentModalChannel = channel;
  state.currentModalFetchIndex++;
  const fetchIndex = state.currentModalFetchIndex;

  // Set basic EPG fields immediately
  document.getElementById("modalTitle").textContent = title;
  document.getElementById("modalTime").textContent = time;
  document.getElementById("modalDesc").textContent = desc || t("modal_no_desc");
  updateModalFavStar();

  // Close fix overlay and reset TMDB UI
  closeFixOverlay();
  resetModalTmdbUi();

  const tmdbLoading = document.getElementById("modalTmdbLoading");
  tmdbLoading.classList.remove("u-hidden");

  document.getElementById("modalOverlay").classList.add("active");

  const lang = getCurrentModalLang();

  try {
    const tmdbData = await invoke("get_tmdb_details", {
      title,
      lang,
    });

    // Check if modal is still open for the same request
    if (
      state.currentModalTitle !== title ||
      fetchIndex !== state.currentModalFetchIndex ||
      !document.getElementById("modalOverlay").classList.contains("active")
    ) {
      return;
    }

    tmdbLoading.classList.add("u-hidden");
    renderTmdbDetailsInModal(tmdbData, { activeLang: lang });
    attachToModal({
      title,
      tmdbId: tmdbData?.tmdbId ?? null,
      mediaType: tmdbData?.mediaType ?? null,
      tmdbData,
    });
  } catch (e) {
    console.warn("TMDB betöltési hiba:", e);
    if (
      state.currentModalTitle === title &&
      fetchIndex === state.currentModalFetchIndex &&
      document.getElementById("modalOverlay").classList.contains("active")
    ) {
      document.getElementById("modalTmdbLoading").classList.add("u-hidden");
      attachToModal({
        title,
        tmdbId: null,
        mediaType: null,
        tmdbData: null,
      });
    }
  }
}

export function updateModalFavStar() {
  const star = document.getElementById("modalFavStar");
  if (!star) return;
  const isFav = state.favorites.includes(state.currentModalTitle);
  if (isFav) {
    star.classList.add("active");
  } else {
    star.classList.remove("active");
  }
  star.innerHTML = iconHtml(isFav ? "star-filled" : "star", 18);
}

export function initModal() {
  document.getElementById("modalFavStar").onclick = () => {
    if (state.currentModalTitle) toggleFavorite(state.currentModalTitle);
  };

  document.getElementById("modalClose").onclick = () => {
    hidePosterZoom();
    closeFixOverlay();
    detachFromModal();
    document.getElementById("modalOverlay").classList.remove("active");
  };

  document.querySelectorAll(".modal-overlay").forEach((overlay) => {
    overlay.onclick = (e) => {
      if (e.target === overlay) {
        if (overlay.id === "modalOverlay") {
          hidePosterZoom();
          closeFixOverlay();
          detachFromModal();
          overlay.classList.remove("active");
        } else if (overlay.id === "settingsOverlay") {
          closeFixOverlay();
          closeSettingsModal();
        } else if (overlay.id === "updateAvailableOverlay") {
          closeUpdateModal();
        } else {
          closeFixOverlay();
          overlay.classList.remove("active");
        }
      }
    };
  });
}
