import { invoke, listen, hasTauri } from "./state.js";
import { t, getCurrentLang, addLanguageChangeListener } from "./i18n.js";
import { formatByteSize, setExportImportStatus } from "./ui.js";

let aboutRepoUrl = "https://github.com/norti/epg-viewer";
let lastCheckResult = null;
let lastCheckResultLang = null;
let isCheckingUpdates = false;
let isInstallingUpdate = false;

function formatReleaseDate(isoStr) {
  if (!isoStr || typeof isoStr !== "string") return "";
  const d = new Date(isoStr);
  if (Number.isNaN(d.getTime())) return isoStr.slice(0, 10);
  const locale = getCurrentLang() === "english" ? "en-US" : "hu-HU";
  return d.toLocaleDateString(locale, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  });
}

function renderAboutChangelog(release) {
  const metaEl = document.getElementById("aboutChangelogMeta");
  const listEl = document.getElementById("aboutChangelogList");
  if (!metaEl || !listEl) return;

  listEl.innerHTML = "";

  if (!release) {
    metaEl.textContent = "";
    const li = document.createElement("li");
    li.className = "about-changelog-empty";
    li.textContent = t("about_no_changelog");
    listEl.appendChild(li);
    return;
  }

  const dateText = formatReleaseDate(release.pubDate);
  metaEl.textContent = dateText
    ? `v${release.version} (${dateText})`
    : `v${release.version}`;

  const items = Array.isArray(release.items) ? release.items : [];
  if (items.length === 0) {
    const li = document.createElement("li");
    li.className = "about-changelog-empty";
    li.textContent = t("about_no_changelog");
    listEl.appendChild(li);
    return;
  }

  for (const item of items) {
    const li = document.createElement("li");
    li.textContent = item;
    listEl.appendChild(li);
  }
}

export async function loadAboutTab() {
  if (!hasTauri()) return;

  try {
    const info = await invoke("get_app_about_info", {
      lang: getCurrentLang(),
    });

    const authorEl = document.getElementById("aboutAuthorValue");
    const versionEl = document.getElementById("aboutVersionValue");
    const repoTextEl = document.getElementById("aboutRepoUrlText");

    if (authorEl) authorEl.textContent = info.author || "norti";
    if (versionEl) versionEl.textContent = `v${info.version}`;
    if (info.repoUrl) {
      aboutRepoUrl = info.repoUrl;
      if (repoTextEl) repoTextEl.textContent = info.repoUrl;
    }

    const currentLang = getCurrentLang();
    const releaseToShow =
      (lastCheckResultLang === currentLang ? lastCheckResult?.latestRelease : null) ||
      info.latestRelease ||
      null;
    renderAboutChangelog(releaseToShow);
  } catch (err) {
    console.error("Failed to load About info:", err);
  }
}

export function closeUpdateModal() {
  if (isInstallingUpdate) return;
  const overlay = document.getElementById("updateAvailableOverlay");
  if (overlay) overlay.classList.remove("active");
}

function openUpdateModal(result) {
  const overlay = document.getElementById("updateAvailableOverlay");
  const pillEl = document.getElementById("updateVersionPill");
  const metaEl = document.getElementById("updateMetaLine");
  const containerEl = document.getElementById("updateChangelogContainer");
  const progressBox = document.getElementById("updateProgressBox");
  const progressFill = document.getElementById("updateProgressFill");
  const progressText = document.getElementById("updateProgressText");
  const errorBox = document.getElementById("updateErrorBox");
  const installBtn = document.getElementById("updateInstallBtn");
  const laterBtn = document.getElementById("updateLaterBtn");
  const closeBtn = document.getElementById("updateModalClose");

  if (!overlay || !containerEl) return;

  if (pillEl) {
    pillEl.textContent = `v${result.currentVersion} → v${result.latestVersion}`;
  }

  if (metaEl) {
    const parts = [];
    const dateStr = formatReleaseDate(result.pubDate);
    if (dateStr) parts.push(`${t("update_release_date")} ${dateStr}`);
    if (result.sizeBytes && result.sizeBytes > 0) {
      parts.push(`${t("update_installer_size")} ${formatByteSize(result.sizeBytes)}`);
    }
    metaEl.textContent = parts.join("  •  ");
    metaEl.classList.toggle("u-hidden", parts.length === 0);
  }

  containerEl.innerHTML = "";
  const releases = Array.isArray(result.releases) ? result.releases : [];
  if (releases.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty-list-message";
    empty.textContent = t("about_no_changelog");
    containerEl.appendChild(empty);
  } else {
    for (const rel of releases) {
      const block = document.createElement("div");
      block.className = "update-release-block";

      const header = document.createElement("div");
      header.className = "update-release-header";

      const verSpan = document.createElement("span");
      verSpan.textContent = `v${rel.version}`;
      header.appendChild(verSpan);

      const relDate = formatReleaseDate(rel.pubDate);
      if (relDate) {
        const dateSpan = document.createElement("span");
        dateSpan.className = "update-release-date";
        dateSpan.textContent = relDate;
        header.appendChild(dateSpan);
      }
      block.appendChild(header);

      const ul = document.createElement("ul");
      ul.className = "update-release-list";
      const items = Array.isArray(rel.items) ? rel.items : [];
      if (items.length === 0) {
        const li = document.createElement("li");
        li.textContent = t("about_no_changelog");
        ul.appendChild(li);
      } else {
        for (const item of items) {
          const li = document.createElement("li");
          li.textContent = item;
          ul.appendChild(li);
        }
      }
      block.appendChild(ul);
      containerEl.appendChild(block);
    }
  }

  if (progressBox) progressBox.classList.add("u-hidden");
  if (progressFill) progressFill.style.width = "0%";
  if (progressText) progressText.textContent = "";
  if (errorBox) {
    errorBox.classList.add("u-hidden");
    errorBox.textContent = "";
  }
  if (installBtn) installBtn.disabled = false;
  if (laterBtn) laterBtn.disabled = false;
  if (closeBtn) closeBtn.disabled = false;

  overlay.classList.add("active");
}

async function startUpdateInstall() {
  if (isInstallingUpdate || !hasTauri()) return;

  const progressBox = document.getElementById("updateProgressBox");
  const progressFill = document.getElementById("updateProgressFill");
  const progressText = document.getElementById("updateProgressText");
  const errorBox = document.getElementById("updateErrorBox");
  const installBtn = document.getElementById("updateInstallBtn");
  const laterBtn = document.getElementById("updateLaterBtn");
  const closeBtn = document.getElementById("updateModalClose");

  isInstallingUpdate = true;
  if (installBtn) installBtn.disabled = true;
  if (laterBtn) laterBtn.disabled = true;
  if (closeBtn) closeBtn.disabled = true;
  if (errorBox) {
    errorBox.classList.add("u-hidden");
    errorBox.textContent = "";
  }
  if (progressBox) progressBox.classList.remove("u-hidden");
  if (progressFill) progressFill.style.width = "0%";
  if (progressText) progressText.textContent = t("update_downloading");

  let unlistenProgress = () => {};
  let unlistenFinished = () => {};

  try {
    unlistenProgress = await listen("update-download-progress", (event) => {
      const payload = event?.payload || {};
      const downloaded = Number(payload.downloaded || 0);
      const total = payload.total ? Number(payload.total) : 0;
      if (total > 0) {
        const pct = Math.min(100, Math.round((downloaded / total) * 100));
        if (progressFill) progressFill.style.width = `${pct}%`;
        if (progressText) {
          progressText.textContent = `${t("update_downloading")} ${pct}% (${formatByteSize(downloaded)} / ${formatByteSize(total)})`;
        }
      } else if (progressText) {
        progressText.textContent = `${t("update_downloading")} (${formatByteSize(downloaded)})`;
      }
    });

    unlistenFinished = await listen("update-download-finished", () => {
      if (progressFill) progressFill.style.width = "100%";
      if (progressText) progressText.textContent = t("update_installing");
    });

    await invoke("install_available_update");
  } catch (err) {
    if (errorBox) {
      errorBox.textContent = String(err);
      errorBox.classList.remove("u-hidden");
    }
    if (progressBox) progressBox.classList.add("u-hidden");
    if (installBtn) installBtn.disabled = false;
    if (laterBtn) laterBtn.disabled = false;
    if (closeBtn) closeBtn.disabled = false;
    isInstallingUpdate = false;
  } finally {
    try {
      unlistenProgress();
    } catch (_) {
      // ignore cleanup error
    }
    try {
      unlistenFinished();
    } catch (_) {
      // ignore cleanup error
    }
  }
}

export async function checkForUpdates({ force = true, silent = false } = {}) {
  if (!hasTauri() || isCheckingUpdates) return null;

  const checkBtn = document.getElementById("checkUpdatesBtn");
  isCheckingUpdates = true;
  if (!silent && checkBtn) checkBtn.disabled = true;
  if (!silent) {
    setExportImportStatus("aboutUpdateStatus", t("about_checking"), "loading");
  }

  try {
    const result = await invoke("check_for_updates", {
      force,
      lang: getCurrentLang(),
    });

    if (!result || !result.checked) {
      return result;
    }

    lastCheckResult = result;
    lastCheckResultLang = getCurrentLang();

    if (result.updateAvailable) {
      if (!silent) {
        setExportImportStatus(
          "aboutUpdateStatus",
          t("about_update_available", { version: result.latestVersion }),
          "success",
        );
      }
      if (result.latestRelease) {
        renderAboutChangelog(result.latestRelease);
      }
      openUpdateModal(result);
    } else {
      if (!silent) {
        setExportImportStatus(
          "aboutUpdateStatus",
          t("about_up_to_date"),
          "success",
        );
      }
      if (result.latestRelease) {
        renderAboutChangelog(result.latestRelease);
      }
    }

    return result;
  } catch (err) {
    if (!silent) {
      setExportImportStatus(
        "aboutUpdateStatus",
        `${t("about_check_error")} ${err}`,
        "error",
      );
    } else {
      console.warn("[updater] Silent startup update check failed:", err);
    }
    return null;
  } finally {
    isCheckingUpdates = false;
    if (!silent && checkBtn) checkBtn.disabled = false;
  }
}

export function initUpdater() {
  const checkBtn = document.getElementById("checkUpdatesBtn");
  if (checkBtn) {
    checkBtn.onclick = () => checkForUpdates({ force: true, silent: false });
  }

  const repoBtn = document.getElementById("aboutRepoLinkBtn");
  if (repoBtn) {
    repoBtn.onclick = () => {
      if (hasTauri() && aboutRepoUrl) {
        invoke("open_url", { url: aboutRepoUrl }).catch((err) =>
          console.error("Failed to open repo URL:", err),
        );
      }
    };
  }

  const closeBtn = document.getElementById("updateModalClose");
  if (closeBtn) closeBtn.onclick = closeUpdateModal;

  const laterBtn = document.getElementById("updateLaterBtn");
  if (laterBtn) laterBtn.onclick = closeUpdateModal;

  const installBtn = document.getElementById("updateInstallBtn");
  if (installBtn) installBtn.onclick = startUpdateInstall;

  addLanguageChangeListener(() => {
    const aboutPane = document.getElementById("tabPaneAbout");
    if (aboutPane && aboutPane.classList.contains("active")) {
      loadAboutTab();
    }
  });
}

export function scheduleStartupUpdateCheck() {
  if (!hasTauri()) return;
  setTimeout(() => {
    checkForUpdates({ force: false, silent: true });
  }, 4000);
}
