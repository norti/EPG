# Privacy Policy — EPG Viewer

**Effective Date:** November 27, 2026  
**Last Updated:** November 27, 2026  
**Application:** EPG Viewer (Windows Desktop Application)  
**Developer:** norti  
**Repository / Contact:** https://github.com/norti/epg-viewer

---

## 1. Overview

EPG Viewer is a standalone, open-source desktop Electronic Program Guide (EPG) viewer application that runs locally on your computer. We respect your privacy and designed the application so that **no personal data is collected by, transmitted to, or stored on any server operated by the developer**.

All application data (EPG listings, channel preferences, favorite programmes, cached images, and encrypted credentials) is stored locally on your device in local SQLite databases.

---

## 2. Information Accessed and How It Is Used

### 2.1. Local Application Data
EPG Viewer stores the following data locally on your computer:
- User-configured EPG XML/GZ source URLs and downloaded TV schedule data.
- Channel ordering, sort presets, hidden/favorite channel flags, custom names, and cached channel logos/posters.
- Favorite programme titles and UI preferences (language, layout, grid scale).

### 2.2. Optional Google Account Integration (OAuth 2.0)
Signing in with a Google account is **completely optional**. If you choose to connect your Google account, EPG Viewer requests only the minimum set of OAuth 2.0 scopes needed for the features you explicitly use:

| Google OAuth Scope | Purpose |
|---|---|
| `openid`, `userinfo.profile`, `userinfo.email` | Displays your Google account name, email address, and profile avatar inside Settings → Google Account so you can see which account is connected. |
| `https://www.googleapis.com/auth/drive.appdata` | Saves and restores a single application backup file (`epg_viewer_sync.json`) containing your non-sensitive EPG Viewer settings, sources, channel settings, sort presets, and favorites in Google Drive's hidden, app-specific **`appDataFolder`**. **EPG Viewer cannot view, list, read, or modify any other files in your Google Drive.** |
| `https://www.googleapis.com/auth/calendar.calendarlist.readonly` | Lists your writable Google Calendars (`owner` / `writer`) so you can select which calendar should receive TV programme reminders. |
| `https://www.googleapis.com/auth/calendar.events` | Creates, tracks/synchronizes, and deletes TV programme reminder events in your selected Google Calendar at your request. EPG Viewer tags every event it creates with a private application marker (`extendedProperties.private.epgViewer = "1"`) and filters synchronization and bulk cleanup strictly to events created by EPG Viewer; your other calendar events are never modified or deleted. |

**Sensitive credentials excluded from cloud backups:** OAuth tokens and your TMDB API key are encrypted locally using Windows DPAPI and are **never** included in Google Drive backups or settings exports.

### 2.3. Optional Third-Party Services
If you choose to configure or use them, EPG Viewer communicates directly from your device with:
- **TMDB (The Movie Database):** Queries movie/TV metadata and posters using your own TMDB API key.
- **Trakt.tv:** Synchronizes your Watchlist, watch history, and TV calendar via Trakt OAuth 2.0 if you choose to sign in.
- **Public logo repositories (tv-logo/tv-logos, iptv-org/database, Wikimedia Commons):** Retrieves TV channel logos on your request.
- **Application update service (`epg.norti.workers.dev`):** Checks a static release manifest (`latest.json` / `changelog.json`) for new application versions. No user identifiers or personal data are sent.

---

## 3. Google API Services User Data Policy — Limited Use Disclosure

EPG Viewer's use and transfer to any other app of information received from Google APIs will adhere to the [Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy), including the **Limited Use** requirements.

Specifically:
- Google user data is used **solely** to provide user-facing features within EPG Viewer (displaying your connected profile, backing up/restoring your app configuration in `appDataFolder`, selecting a target calendar, and creating/syncing/removing your TV programme calendar events).
- We do **not** transfer, sell, or share Google user data with third parties, advertisers, or data brokers.
- We do **not** use Google user data for serving advertisements, personalized advertising, or retargeting.
- We do **not** use Google user data to train generalized artificial intelligence (AI) or machine learning (ML) models.
- No human (including the developer) reads or has access to your Google user data, because all API communication occurs directly between your local device and Google's servers.

---

## 4. Data Storage and Security

- **Local-only token storage:** Google OAuth `access_token` and `refresh_token` values are encrypted on disk using Windows Data Protection API (**DPAPI**, `CryptProtectData`) tied to your Windows user account, stored in a dedicated local SQLite table (`google_auth`), and zeroed from memory when dropped.
- **PKCE & Loopback OAuth:** Desktop sign-in uses RFC 8252 OAuth 2.0 for Native Apps with S256 PKCE, a cryptographic CSRF `state` nonce, and an ephemeral `127.0.0.1` loopback listener.
- **Transport security:** All communication with Google APIs uses HTTPS (TLS).

---

## 5. Data Retention and Deletion

You retain full control over your data at all times:

1. **Disconnect / Sign out in EPG Viewer:** Go to **Settings → Google Account → Sign out**. EPG Viewer revokes your OAuth token with Google (`https://oauth2.googleapis.com/revoke`) and immediately deletes locally stored Google tokens, cached profile information, sync state, and locally tracked calendar event references.
2. **Delete calendar events created by EPG Viewer:** Remove individual events from any programme detail modal, or use **Settings → Google Account → Google Calendar Settings → Delete** (older than X days or all) to remove EPG Viewer-created events from both your local database and Google Calendar.
3. **Delete Google Drive backup data:** Open [Google Drive](https://drive.google.com) → **Settings → Manage Apps → EPG Viewer → Options → Delete hidden app data**.
4. **Revoke Google permissions externally:** You can revoke EPG Viewer's access at any time from your Google Account permissions page: https://myaccount.google.com/permissions.
5. **Delete all local data:** Uninstall EPG Viewer and delete the local application data folder (`%APPDATA%\epg-viewer`).

---

## 6. Children's Privacy

EPG Viewer does not knowingly collect any personal information from anyone, including children under the age of 13 (or the applicable age of digital consent in your jurisdiction).

---

## 7. Changes to This Privacy Policy

We may update this Privacy Policy from time to time to reflect changes in application features or legal requirements. Updates will be published in this repository with a revised "Last Updated" date.

---

## 8. Contact

If you have questions or concerns about this Privacy Policy or data handling in EPG Viewer, please open an issue or discussion in the official GitHub repository:

- **GitHub:** https://github.com/norti/epg-viewer/issues
