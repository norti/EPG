window.setModalContext = function (title, channel) {
  currentModalTitle = title || "";
  currentModalChannel = channel || null;
  currentModalFetchIndex++;
};

async function openModal(title, time, desc, channel) {
  currentModalTitle = title;
  currentModalChannel = channel;
  currentModalFetchIndex++;
  const fetchIndex = currentModalFetchIndex;

  // Set basic EPG fields immediately
  document.getElementById("modalTitle").textContent = title;
  document.getElementById("modalTime").textContent = time;
  document.getElementById("modalDesc").textContent = desc || t("modal_no_desc");
  updateModalFavStar();

  // Close fix overlay and reset TMDB UI
  closeFixOverlay();
  resetModalTmdbUi();

  const tmdbLoading = document.getElementById("modalTmdbLoading");
  tmdbLoading.style.display = "flex";

  document.getElementById("modalOverlay").classList.add("active");

  const lang = getCurrentModalLang();

  try {
    const tmdbData = await invoke("get_tmdb_details", {
      title,
      lang,
    });

    // Check if modal is still open for the same request
    if (
      currentModalTitle !== title ||
      fetchIndex !== currentModalFetchIndex ||
      !document.getElementById("modalOverlay").classList.contains("active")
    ) {
      return;
    }

    tmdbLoading.style.display = "none";
    renderTmdbDetailsInModal(tmdbData);
    if (window.TraktUI) {
      window.TraktUI.attachToModal({
        title,
        tmdbId: tmdbData?.tmdbId ?? null,
        mediaType: tmdbData?.mediaType ?? null,
        tmdbData,
      });
    }
  } catch (e) {
    console.warn("TMDB betöltési hiba:", e);
    if (currentModalTitle === title) {
      document.getElementById("modalTmdbLoading").style.display = "none";
      if (window.TraktUI) {
        window.TraktUI.attachToModal({
          title,
          tmdbId: null,
          mediaType: null,
          tmdbData: null,
        });
      }
    }
  }
}

function updateModalFavStar() {
  const star = document.getElementById("modalFavStar");
  if (!star) return;
  const isFav = favorites.includes(currentModalTitle);
  if (isFav) {
    star.classList.add("active");
  } else {
    star.classList.remove("active");
  }
  star.innerHTML = iconHtml(isFav ? "star-filled" : "star", 18);
}

document.getElementById("modalFavStar").onclick = () => {
  if (currentModalTitle) toggleFavorite(currentModalTitle);
};

document.getElementById("modalClose").onclick = () => {
  hidePosterZoom();
  closeFixOverlay();
  if (window.TraktUI) {
    window.TraktUI.detachFromModal();
  }
  document.getElementById("modalOverlay").classList.remove("active");
};

document.querySelectorAll(".modal-overlay").forEach((overlay) => {
  overlay.onclick = (e) => {
    if (e.target === overlay) {
      closeFixOverlay();
      overlay.classList.remove("active");
      if (
        overlay.id === "settingsOverlay" &&
        currentSourceId === "__ALL_FAVORITES__"
      ) {
        fetchEPG();
      }
    }
  };
});
