# EPG Viewer

[English](README.md) | **Magyar**

Fejlesztés alatt... Az első verzió megjelent.

Ez egy EPG XML fájlokat megjelenítő alkalmazás. Képes .xml vagy .gz EPG fájlokból (URL forrás) TV-műsorújságokat megjeleníteni.
A csatornákhoz automatikusan hozzárendeltethetsz csatorna logókat.
A műsorokhoz és filmekhez TMDB információkat kérhetsz le (API kulcsra szükség van).
Összekapcsolhatod a Trakt-fiókoddal, megjelenítheted a Watchlistedet és a naptárat, műsorokat/filmeket adhatsz a Watchlisthez, és megkeresheted őket az EPG-ben.

Ez **NEM** lejátszó/stream alkalmazás, csak egy funkciókban gazdag műsorújság-megjelenítő.

## Letöltés (Windows x64)

- **[Legfrissebb telepítő letöltése (Windows x64)](https://epg.norti.workers.dev/download/latest)** _(Cloudflare R2)_
- **[Változásnapló](CHANGELOG.md)** · **[Kiadások](https://github.com/norti/epg-viewer/releases)**

![](screenshots/epg-viewer_01.png)
![](screenshots/epg-viewer_02.png)
![](screenshots/epg-viewer_03.png)
![](screenshots/epg-viewer_04.png)
![](screenshots/epg-viewer_05.png)
![](screenshots/epg-viewer_06.png)
![](screenshots/epg-viewer_07.png)
![](screenshots/epg-viewer_08.png)
![](screenshots/epg-viewer_09.png)
![](screenshots/epg-viewer_10.png)
![](screenshots/epg-viewer_11.jpg)
![](screenshots/epg-viewer_12.png)
![](screenshots/epg-viewer_13.png)
![](screenshots/epg-viewer_14.png)

---

## Licenc és jogi nyilatkozatok

A szoftver forráskódja az [MIT Licenc](LICENSE) hatálya alá tartozik.

### Harmadik felek védjegyei és csatornalogók

A megjelenített televíziós csatornanevek, logók és márkajelzések a megfelelő műsorszolgáltatók és védjegytulajdonosok tulajdonát képezik. A logók megjelenítése az elektronikus műsorújságban (EPG) kizárólag azonosítási és tájékoztatási célokat szolgál (_nominative fair use_).

A szoftver forráskódja és telepítőcsomagja nem tartalmaz előre becsomagolt csatornalogókat. A logók futásidőben, a felhasználó kérésére kerülnek lekérésre nyilvános külső forrásokból, és kizárólag a felhasználó saját számítógépén (helyi SQLite adatbázisban) kerülnek gyorsítótárazásra.

### Felhasznált külső források

- **tv-logo/tv-logos:** Sötét hátterekre optimalizált TV-csatorna logók (https://github.com/tv-logo/tv-logos)
- **iptv-org/database:** Nyílt IPTV adatbázis és csatornajegyzék (https://github.com/iptv-org/database, CC0 1.0 Universal)
- **Wikimedia Commons:** Szabad médiafájlok és vektoros SVG logók (https://commons.wikimedia.org)
- **Trakt.tv API:** Műsorkövetés, Watchlist, naptár és értékelések (https://trakt.tv). _EPG Viewer uses Trakt but is not endorsed or certified by Trakt._
- **TMDB (The Movie Database):** Film- és sorozatmetaadatok, poszterek és magyar leírások (https://www.themoviedb.org). _This product uses the TMDB API but is not endorsed or certified by TMDB._

### UI ikonok

A felhasználói felületen alkalmazott vektoros ikonok a **Tabler Icons** gyűjteményből származnak (https://tabler.io/icons), melyek az MIT Licenc feltételei szerint használhatók.
