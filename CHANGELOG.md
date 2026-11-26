# Changelog

## v1.1.0 (2026-11-26)

### Magyar
- Önálló Kedvenc műsorok modal ablak (kikerült a Beállításokból), összes EPG forrás lekérdezésével, forrásszűrővel, múltbeli adások kapcsolóval, ugrás gombbal és TMDB információs ablakkal a program nevére klikkelve
- Új Trakt 'Watchlist az EPG-n' nézet az összes EPG forrásban szereplő Watchlist műsorhoz (alapértelmezett A-Z rendezés, forrás/csatorna/dátum/nap/idő badge-ek, ugrás, kedvenc csillag, azonnali Watchlist törlés és TMDB információs ablakkal a program nevére klikkelve)
- Max. 3 szintű rendezés (Dátum, A-Z, Csatorna, Forrás) és az 1. szint szerint összecsukható/kibontható szakaszok Mind becsukása / Mind kinyitása gombbal
- Konzisztens EPG / Trakt módváltó gomb a fejléc jobb oldalán azonos mérettel, valamint egységes élő keresőmezők belső X gombbal és ESC törléssel az egész alkalmazásban
- Finomított modal-kezelés (a Beállítások és a Csatorna szerkesztése modal nem záródik be ESC-re vagy mellékattintásra) és egységesített kedvenc csillag / dátum badge stílusok

### English
- Standalone Favorite Programmes modal (moved out of Settings), querying all EPG sources with source filter, past airings toggle, jump-to-EPG button, and TMDB modal support
- New Trakt 'Watchlist on EPG' view, listing all Watchlist programmes across local EPG sources (default A-Z sort, source/channel/date/day/time badges, jump to EPG, favorite toggle, instant Watchlist removal, and TMDB modal)
- Up to 3 level sorting (Date, A-Z, Channel, Source) with collapsible sections by primary sort level and a Collapse all / Expand all toggle button
- Consistent EPG / Trakt mode toggle button on the right side of the header with matching dimensions, plus unified live search inputs with inline X clear button and ESC clearing across the app
- Refined modal behavior (Settings and Edit Channel modals no longer close on ESC or backdrop click) and unified favorite star / date badge styling

## v1.0.0 (2026-11-25)

### Magyar
- Első stabil kiadás: XML/GZ EPG műsorújság megjelenítő függőleges és vízszintes nézettel
- TMDB és Trakt.tv integráció (poszterek, leírások, Watchlist, naptár, megtekintési előzmények)
- Automatikus csatornalogó-keresés és duplikátum-kezelés helyi SQLite gyorsítótárral
- Magyar és angol nyelvű felület, beállítások és csatornaadatok exportálása/importálása

### English
- Initial stable release: XML/GZ EPG TV guide viewer with vertical and horizontal layouts
- TMDB and Trakt.tv integration (posters, descriptions, Watchlist, calendar, watch history)
- Automatic channel logo matching and deduplication with local SQLite image cache
- Hungarian and English UI with settings and channel data export/import
