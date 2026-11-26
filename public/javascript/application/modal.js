import { state, invoke } from "./state.js";
import { t } from "./i18n.js";
import { iconHtml } from "./ui.js";
import { toggleFavorite } from "./favorites.js";
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

export async function openModal(title, time, desc, channel, options = {}) {
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

  const isStillCurrent = () =>
    state.currentModalTitle === title &&
    fetchIndex === state.currentModalFetchIndex &&
    document.getElementById("modalOverlay").classList.contains("active");

  try {
    let tmdbData = await invoke("get_tmdb_details", {
      title,
      lang,
    });

    if (!isStillCurrent()) return;

    if (!tmdbData && options && options.fallbackTmdbId) {
      try {
        tmdbData = await invoke("get_tmdb_details_by_id", {
          tmdbId: Number(options.fallbackTmdbId),
          mediaType: options.fallbackMediaType || "tv",
          lang,
        });
      } catch (_) {
        // Ignore fallback lookup failure and continue with Trakt attachment
      }
      if (!isStillCurrent()) return;
    }

    tmdbLoading.classList.add("u-hidden");
    renderTmdbDetailsInModal(tmdbData, { activeLang: lang });
    attachToModal({
      title,
      tmdbId: tmdbData?.tmdbId ?? options?.fallbackTmdbId ?? null,
      mediaType: tmdbData?.mediaType ?? options?.fallbackMediaType ?? null,
      tmdbData,
    });
  } catch (e) {
    console.warn("TMDB betöltési hiba:", e);
    if (isStillCurrent()) {
      document.getElementById("modalTmdbLoading").classList.add("u-hidden");
      attachToModal({
        title,
        tmdbId: options?.fallbackTmdbId ?? null,
        mediaType: options?.fallbackMediaType ?? null,
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

  document.addEventListener("keydown", (e) => {
    if (e.key !== "Escape") return;
    const confirmOverlay = document.getElementById("customConfirmOverlay");
    const promptOverlay = document.getElementById("customPromptOverlay");
    const logoSearchModal = document.getElementById("logoSearchModal");
    const missingLogosModal = document.getElementById("missingLogosModal");
    const channelEditOverlay = document.getElementById("channelEditModalOverlay");
    if (
      confirmOverlay?.classList.contains("active") ||
      promptOverlay?.classList.contains("active") ||
      logoSearchModal?.classList.contains("active") ||
      missingLogosModal?.classList.contains("active") ||
      channelEditOverlay?.classList.contains("active")
    ) {
      return;
    }

    const fixOverlay = document.getElementById("modalFixOverlay");
    const modalOverlay = document.getElementById("modalOverlay");
    if (modalOverlay?.classList.contains("active")) {
      if (fixOverlay && !fixOverlay.classList.contains("u-hidden")) {
        closeFixOverlay();
      } else {
        hidePosterZoom();
        closeFixOverlay();
        detachFromModal();
        modalOverlay.classList.remove("active");
      }
      return;
    }

    const favOverlay = document.getElementById("favoritesModalOverlay");
    if (favOverlay?.classList.contains("active")) {
      favOverlay.classList.remove("active");
    }
  });

  document.querySelectorAll(".modal-overlay").forEach((overlay) => {
    overlay.onclick = (e) => {
      if (e.target === overlay) {
        if (
          overlay.id === "settingsOverlay" ||
          overlay.id === "channelEditModalOverlay"
        ) {
          return;
        }
        if (overlay.id === "modalOverlay") {
          hidePosterZoom();
          closeFixOverlay();
          detachFromModal();
          overlay.classList.remove("active");
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
