/**
 * Tabler Icons segédprogram az EPG Viewer projekthez
 *
 * Használat:
 *   node tools/update-icons.js build         -> Újraépíti a public/javascript/icons.js fájlt az alapértelmezett ikonokkal
 *   node tools/update-icons.js add <név...>   -> Új ikont ad a public/javascript/icons.js-hez a helyi icons/tabler-icons/ gyűjteményből
 *   node tools/update-icons.js search <kulcsszó> -> Keresés a helyi >6000 Tabler SVG ikon között
 *   node tools/update-icons.js list          -> Kilistázza a public/javascript/icons.js-ben elérhető ikonokat
 */

const fs = require('fs');
const path = require('path');

const ROOT_DIR = path.resolve(__dirname, '..');
const OUTLINE_DIR = path.join(ROOT_DIR, 'icons', 'tabler-icons', 'outline');
const FILLED_DIR = path.join(ROOT_DIR, 'icons', 'tabler-icons', 'filled');
const OUTPUT_FILE = path.join(ROOT_DIR, 'public', 'javascript', 'icons.js');

// Az alkalmazásban használt és a PRE-PLAN 2.1 alapján szükséges ikonok alapértelmezett listája
const DEFAULT_ICONS = [
  'star',
  'star-filled',
  'chevron-left',
  'chevron-right',
  'layout-grid',
  'table',
  'x',
  'settings',
  'home',
  'bolt',
  'eye',
  'eye-off',
  'folder',
  'device-tv',
  'photo',
  'photo-search',
  'movie',
  'arrows-exchange',
  'ban',
  'arrow-up',
  'arrow-down',
  'pencil',
  'dots',
  'trash',
  'calendar',
  'search',
  'check',
  'refresh',
  'reload',
  'download',
  'info-circle',
  'alert-triangle'
];

function findIconSvg(iconName) {
  let isFilled = false;
  let targetName = iconName;

  if (iconName.endsWith('-filled')) {
    const base = iconName.replace(/-filled$/, '');
    const filledPath = path.join(FILLED_DIR, `${base}.svg`);
    if (fs.existsSync(filledPath)) {
      return { path: filledPath, isFilled: true };
    }
  }

  const outlinePath = path.join(OUTLINE_DIR, `${targetName}.svg`);
  if (fs.existsSync(outlinePath)) {
    return { path: outlinePath, isFilled: false };
  }

  const filledPath = path.join(FILLED_DIR, `${targetName}.svg`);
  if (fs.existsSync(filledPath)) {
    return { path: filledPath, isFilled: true };
  }

  return null;
}

function extractSvgInner(svgContent) {
  const match = svgContent.match(/<svg[^>]*>([\s\S]*?)<\/svg>/i);
  if (!match) return '';
  return match[1].trim().replace(/\r?\n\s*/g, '');
}

function generateIconsJs(iconNames) {
  const uniqueNames = Array.from(new Set(iconNames)).sort();
  const iconData = {};

  for (const name of uniqueNames) {
    const found = findIconSvg(name);
    if (!found) {
      console.warn(`[Figyelem] Ikon nem található: "${name}"`);
      continue;
    }
    const content = fs.readFileSync(found.path, 'utf-8');
    const inner = extractSvgInner(content);
    iconData[name] = {
      paths: inner,
      filled: found.isFilled
    };
  }

  const fileContent = `/**
 * Tabler Icons (MIT License) - EPG Viewer Inline SVG Helper
 * 
 * Auto-generálva a tools/update-icons.js script segítségével.
 * Ikonok forrása: Tabler Icons (https://tabler.io/icons) - Copyright (c) Paweł Kuna (MIT)
 */

(function () {
  'use strict';

  const ICONS = ${JSON.stringify(iconData, null, 2)};

  const AppIcons = {
    /**
     * Visszaadja a megadott nevű inline SVG HTML szöveget.
     * @param {string} name - Az ikon neve (pl. 'star', 'star-filled', 'settings', 'device-tv')
     * @param {object} options - Opcionális beállítások: { size = 20, className = '', strokeWidth = 2, fill = null }
     * @returns {string} Inline SVG markup
     */
    get(name, options = {}) {
      const def = ICONS[name];
      if (!def) {
        console.warn('[Icons] Nem található ikon: ' + name);
        return '';
      }

      const size = options.size !== undefined ? options.size : 20;
      const strokeWidth = options.strokeWidth !== undefined ? options.strokeWidth : 2;
      const className = options.className ? ' ' + options.className : '';
      const isFilled = def.filled;

      const fillAttr = options.fill || (isFilled ? 'currentColor' : 'none');
      const strokeAttr = isFilled ? 'none' : 'currentColor';
      const strokeWidthAttr = isFilled ? '0' : strokeWidth;

      return '<svg xmlns="http://www.w3.org/2000/svg" width="' + size + '" height="' + size + '" ' +
        'viewBox="0 0 24 24" fill="' + fillAttr + '" stroke="' + strokeAttr + '" ' +
        'stroke-width="' + strokeWidthAttr + '" stroke-linecap="round" stroke-linejoin="round" ' +
        'class="tabler-icon tabler-icon-' + name + className + '">' +
        def.paths +
        '</svg>';
    },

    /**
     * DOM elemként példányosítja az ikont (SVGElement).
     * @param {string} name
     * @param {object} options
     * @returns {SVGElement|null}
     */
    render(name, options = {}) {
      const html = this.get(name, options);
      if (!html) return null;
      const template = document.createElement('template');
      template.innerHTML = html.trim();
      return template.content.firstChild;
    },

    /**
     * Automatikusan lecseréli a [data-icon="..."] attribútumú elemek tartalmát inline SVG-re.
     * @param {HTMLElement|Document} root
     */
    replace(root = document) {
      if (!root || !root.querySelectorAll) return;
      const elements = root.querySelectorAll('[data-icon]');
      elements.forEach((el) => {
        const iconName = el.getAttribute('data-icon');
        if (!iconName) return;
        const size = el.getAttribute('data-icon-size') ? parseInt(el.getAttribute('data-icon-size'), 10) : undefined;
        const strokeWidth = el.getAttribute('data-icon-stroke') ? parseFloat(el.getAttribute('data-icon-stroke')) : undefined;
        const className = el.getAttribute('data-icon-class') || '';
        el.innerHTML = this.get(iconName, { size, strokeWidth, className });
      });
    },

    /**
     * Elérhető ikonok listája
     * @returns {string[]}
     */
    list() {
      return Object.keys(ICONS);
    },

    /**
     * Ellenőrzi, hogy egy adott ikon elérhető-e a könyvtárban
     * @param {string} name
     * @returns {boolean}
     */
    has(name) {
      return Object.prototype.hasOwnProperty.call(ICONS, name);
    },

    /**
     * Dinamikus új ikon regisztrálása futásidőben
     * @param {string} name
     * @param {string} paths
     * @param {boolean} filled
     */
    register(name, paths, filled = false) {
      ICONS[name] = { paths, filled };
    }
  };

  window.Icons = AppIcons;
})();
`;

  fs.writeFileSync(OUTPUT_FILE, fileContent, 'utf-8');
  console.log(`[Siker] ${Object.keys(iconData).length} ikon sikeresen beépítve ide: ${OUTPUT_FILE}`);
}

// Fővezérlő
const args = process.argv.slice(2);
const command = args[0] || 'build';

if (command === 'build') {
  console.log('Alapértelmezett ikonok generálása...');
  generateIconsJs(DEFAULT_ICONS);
} else if (command === 'add') {
  const newNames = args.slice(1);
  if (newNames.length === 0) {
    console.error('Hiba: Nem adtál meg ikonnevet! Példa: node tools/update-icons.js add volume microphone');
    process.exit(1);
  }
  // Olvassuk be a meglévőket
  let currentList = [...DEFAULT_ICONS];
  if (fs.existsSync(OUTPUT_FILE)) {
    const content = fs.readFileSync(OUTPUT_FILE, 'utf-8');
    const match = content.match(/const ICONS = ([\s\S]*?);/);
    if (match) {
      try {
        const parsed = JSON.parse(match[1]);
        currentList = Object.keys(parsed);
      } catch (e) {}
    }
  }
  for (const n of newNames) {
    if (!currentList.includes(n)) currentList.push(n);
  }
  generateIconsJs(currentList);
} else if (command === 'list') {
  if (fs.existsSync(OUTPUT_FILE)) {
    const content = fs.readFileSync(OUTPUT_FILE, 'utf-8');
    const match = content.match(/const ICONS = ([\s\S]*?);/);
    if (match) {
      const parsed = JSON.parse(match[1]);
      const keys = Object.keys(parsed);
      console.log(`Elérhető ikonok (${keys.length} db):`);
      console.log(keys.join(', '));
    }
  } else {
    console.log('A public/icons.js még nem létezik.');
  }
} else if (command === 'search') {
  const keyword = (args[1] || '').toLowerCase();
  if (!keyword) {
    console.error('Hiba: Nem adtál meg keresőszót! Példa: node tools/update-icons.js search tv');
    process.exit(1);
  }
  const outlineFiles = fs.readdirSync(OUTLINE_DIR).filter(f => f.endsWith('.svg')).map(f => f.replace('.svg', ''));
  const filledFiles = fs.readdirSync(FILLED_DIR).filter(f => f.endsWith('.svg')).map(f => f.replace('.svg', '') + '-filled');
  const all = [...outlineFiles, ...filledFiles];
  const matches = all.filter(name => name.includes(keyword));
  console.log(`Találatok "${keyword}" szóra (${matches.length} db):`);
  console.log(matches.slice(0, 50).join(', '));
  if (matches.length > 50) console.log(`...és még ${matches.length - 50} találat.`);
} else {
  console.log('Ismeretlen parancs. Használat: build, add, list, search');
}
