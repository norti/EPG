const express = require('express');
const axios = require('axios');
const zlib = require('zlib');
const xml2js = require('xml2js');
const fs = require('fs-extra');
const path = require('path');
const Database = require('better-sqlite3');
const cron = require('node-cron');

const app = express();
const PORT = 3000;

const DATA_DIR = path.join(__dirname, 'data');
fs.ensureDirSync(DATA_DIR);

const DB_PATH = path.join(DATA_DIR, 'epg.db');
const SOURCES_FILE = path.join(DATA_DIR, 'sources.json');

app.use(express.json());
app.use(express.static('public'));

const db = new Database(DB_PATH);

db.exec(`
    CREATE TABLE IF NOT EXISTS channels (
        id TEXT,
        source_id TEXT,
        name TEXT,
        icon TEXT,
        PRIMARY KEY (id, source_id)
    );
    CREATE TABLE IF NOT EXISTS programmes (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        channel_id TEXT,
        source_id TEXT,
        start TEXT,
        stop TEXT,
        title TEXT,
        desc TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_prog_time ON programmes (source_id, start, stop);
    CREATE INDEX IF NOT EXISTS idx_prog_channel ON programmes (channel_id);
`);

if (!fs.existsSync(SOURCES_FILE)) {
    fs.writeJsonSync(SOURCES_FILE, [
        {
            id: 'default_hu',
            name: 'EPG Share HU',
            url: 'https://epgshare01.online/epgshare01/epg_ripper_HU1.xml.gz',
            lastDownloaded: null
        }
    ]);
}

cron.schedule('0 3 * * *', async () => {
    console.log('[CRON] Automatikus EPG frissítés indítása...');
    try {
        const sources = await fs.readJson(SOURCES_FILE);
        for (const src of sources) {
            await updateEPGData(src.id);
        }
    } catch (err) {
        console.error('[CRON Hiba]:', err.message);
    }
});

async function updateEPGData(sourceId) {
    const sources = await fs.readJson(SOURCES_FILE);
    const source = sources.find(s => s.id === sourceId);
    if (!source) throw new Error('Forrás nem található');

    console.log(`[EPG Update] Letöltés indítása: ${source.url}`);
    const response = await axios.get(source.url, { responseType: 'arraybuffer' });
    
    let xmlString = '';
    if (source.url.endsWith('.gz')) {
        xmlString = zlib.gunzipSync(response.data).toString('utf-8');
    } else {
        xmlString = response.data.toString('utf-8');
    }

    return new Promise((resolve, reject) => {
        xml2js.parseString(xmlString, { explicitArray: false }, async (err, result) => {
            if (err || !result?.tv) return reject('XML Parzolási hiba');

            const rawChannels = Array.isArray(result.tv.channel) ? result.tv.channel : (result.tv.channel ? [result.tv.channel] : []);
            const rawProgrammes = Array.isArray(result.tv.programme) ? result.tv.programme : (result.tv.programme ? [result.tv.programme] : []);

            try {
                const deleteChannels = db.prepare(`DELETE FROM channels WHERE source_id = ?`);
                const deleteProgrammes = db.prepare(`DELETE FROM programmes WHERE source_id = ?`);
                
                const insertChannel = db.prepare(`INSERT OR REPLACE INTO channels (id, source_id, name, icon) VALUES (?, ?, ?, ?)`);
                const insertProgramme = db.prepare(`INSERT INTO programmes (channel_id, source_id, start, stop, title, desc) VALUES (?, ?, ?, ?, ?, ?)`);

                const insertTransaction = db.transaction(() => {
                    deleteChannels.run(sourceId);
                    deleteProgrammes.run(sourceId);

                    for (const ch of rawChannels) {
                        const name = String(ch['display-name'] ? (ch['display-name']._ || ch['display-name']) : ch.$.id);
                        const icon = ch.icon?.$?.src || '';
                        insertChannel.run(ch.$.id, sourceId, name, icon);
                    }

                    for (const prog of rawProgrammes) {
                        const title = String(prog.title ? (prog.title._ || prog.title) : 'Nincs cím');
                        const desc = String(prog.desc ? (prog.desc._ || prog.desc) : '');
                        insertProgramme.run(
                            prog.$.channel,
                            sourceId,
                            parseEPGDate(prog.$.start),
                            parseEPGDate(prog.$.stop),
                            title,
                            desc
                        );
                    }
                });

                insertTransaction();

                source.lastDownloaded = new Date().toISOString();
                await fs.writeJson(SOURCES_FILE, sources);

                console.log(`[EPG Update] Sikeresen frissítve az SQLite-ban: ${source.name}`);
                resolve();
            } catch (dbErr) {
                reject(dbErr);
            }
        });
    });
}

function parseEPGDate(str) {
    if (!str) return null;
    const y = str.substring(0, 4);
    const m = str.substring(4, 6) - 1;
    const d = str.substring(6, 8);
    const h = str.substring(8, 10);
    const min = str.substring(10, 12);
    const s = str.substring(12, 14);
    return new Date(Date.UTC(y, m, d, h, min, s)).toISOString();
}

app.get('/api/sources', async (req, res) => {
    const sources = await fs.readJson(SOURCES_FILE);
    res.json(sources);
});

app.post('/api/sources', async (req, res) => {
    const { id, name, url } = req.body;
    let sources = await fs.readJson(SOURCES_FILE);

    if (id) {
        sources = sources.map(s => s.id === id ? { ...s, name, url } : s);
    } else {
        sources.push({ id: 'src_' + Date.now(), name, url, lastDownloaded: null });
    }

    await fs.writeJson(SOURCES_FILE, sources);
    res.json({ success: true });
});

app.delete('/api/sources/:id', async (req, res) => {
    const { id } = req.params;
    let sources = await fs.readJson(SOURCES_FILE);
    sources = sources.filter(s => s.id !== id);

    db.prepare(`DELETE FROM channels WHERE source_id = ?`).run(id);
    db.prepare(`DELETE FROM programmes WHERE source_id = ?`).run(id);

    await fs.writeJson(SOURCES_FILE, sources);
    res.json({ success: true });
});

app.post('/api/sources/:id/download', async (req, res) => {
    try {
        await updateEPGData(req.params.id);
        res.json({ success: true });
    } catch (err) {
        console.error(err);
        res.status(500).json({ error: err.toString() });
    }
});

app.get('/api/epg', async (req, res) => {
    const sourceId = req.query.sourceId;
    const sources = await fs.readJson(SOURCES_FILE);
    const source = sources.find(s => s.id === sourceId) || sources[0];

    if (!source) return res.status(404).json({ error: 'Nincs forrás' });

    try {
        let channels = db.prepare(`SELECT id, name, icon FROM channels WHERE source_id = ?`).all(source.id);
        
        if (channels.length === 0) {
            console.log(`[EPG Auto-Fetch] Üres adatbázis, letöltés indítása: ${source.name}...`);
            await updateEPGData(source.id);
            channels = db.prepare(`SELECT id, name, icon FROM channels WHERE source_id = ?`).all(source.id);
        }

        const rawProgrammes = db.prepare(`SELECT channel_id as channel, start, stop, title, desc FROM programmes WHERE source_id = ?`).all(source.id);

        res.json({ channels, programmes: rawProgrammes, sourceName: source.name });
    } catch (err) {
        console.error(err);
        res.status(500).json({ error: err.message });
    }
});

app.listen(PORT, () => console.log(`Server: http://localhost:${PORT}`));