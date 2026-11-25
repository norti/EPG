import fs from "node:fs";
import path from "node:path";

const BROWSER_GLOBALS = Object.fromEntries(
  [
    "window",
    "document",
    "console",
    "navigator",
    "location",
    "history",
    "localStorage",
    "sessionStorage",
    "performance",
    "fetch",
    "Headers",
    "Request",
    "Response",
    "URL",
    "URLSearchParams",
    "Blob",
    "File",
    "FileReader",
    "FormData",
    "Image",
    "DOMParser",
    "XMLSerializer",
    "AbortController",
    "AbortSignal",
    "Event",
    "CustomEvent",
    "MouseEvent",
    "KeyboardEvent",
    "FocusEvent",
    "InputEvent",
    "Node",
    "NodeFilter",
    "Element",
    "HTMLElement",
    "HTMLInputElement",
    "SVGElement",
    "MutationObserver",
    "ResizeObserver",
    "IntersectionObserver",
    "CSS",
    "getComputedStyle",
    "matchMedia",
    "requestAnimationFrame",
    "cancelAnimationFrame",
    "setTimeout",
    "clearTimeout",
    "setInterval",
    "clearInterval",
    "queueMicrotask",
    "structuredClone",
    "atob",
    "btoa",
    "crypto",
    "alert",
    "confirm",
    "prompt",
    "innerWidth",
    "innerHeight",
    "devicePixelRatio",
    "scrollTo",
    "scrollBy",
  ].map((name) => [name, "readonly"]),
);

const NODE_GLOBALS = Object.fromEntries(
  [
    "process",
    "Buffer",
    "console",
    "__dirname",
    "__filename",
    "module",
    "exports",
    "require",
    "setTimeout",
    "clearTimeout",
    "setInterval",
    "clearInterval",
    "setImmediate",
    "clearImmediate",
    "URL",
    "URLSearchParams",
    "TextEncoder",
    "TextDecoder",
  ].map((name) => [name, "readonly"]),
);

/**
 * Extracts top-level identifiers exposed into the shared browser global scope by a classic
 * `<script>` file (top-level function/var/let/const/class declarations and `window.X =` / `root.X =`).
 */
function extractScriptGlobals(filePath) {
  const src = fs.readFileSync(filePath, "utf8");
  const topLevelDecls = new Set();
  const windowExports = new Set();

  // Top-level declarations (column 0, outside functions/blocks)
  const declRe = /^(?:async\s+)?(?:function|class|var|let|const)\s+([A-Za-z_$][A-Za-z0-9_$]*)/gm;
  for (const m of src.matchAll(declRe)) {
    topLevelDecls.add(m[1]);
  }

  // Top-level object destructuring declarations (e.g. `const { invoke } = window.__TAURI__.core;`)
  const destructRe = /^(?:var|let|const)\s*\{([^}]+)\}\s*=/gm;
  for (const m of src.matchAll(destructRe)) {
    for (const part of m[1].split(",")) {
      const cleaned = part.trim().split("=")[0].trim();
      const binding = cleaned.includes(":") ? cleaned.split(":")[1].trim() : cleaned;
      if (/^[A-Za-z_$][A-Za-z0-9_$]*$/.test(binding)) {
        topLevelDecls.add(binding);
      }
    }
  }

  // Explicit global exports onto window / UMD root
  const exportRe = /\b(?:window|root)\.([A-Za-z_$][A-Za-z0-9_$]*)\s*=(?!=)/g;
  for (const m of src.matchAll(exportRe)) {
    windowExports.add(m[1]);
  }

  return { topLevelDecls, windowExports };
}

const indexHtml = fs.readFileSync(path.resolve("public/index.html"), "utf8");
const scriptFiles = [...indexHtml.matchAll(/<script\s+src="([^"]+)"><\/script>/g)].map((m) =>
  path.posix.join("public", m[1]),
);

const declaredByFile = new Map(
  scriptFiles.map((file) => [file, extractScriptGlobals(path.resolve(file))]),
);

const browserScriptConfigs = scriptFiles.map((file) => {
  const scriptGlobals = {};
  for (const [otherFile, { topLevelDecls, windowExports }] of declaredByFile.entries()) {
    for (const name of windowExports) {
      scriptGlobals[name] = "writable";
    }
    if (otherFile === file) continue;
    for (const name of topLevelDecls) {
      scriptGlobals[name] = "writable";
    }
  }
  return {
    files: [file],
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: "script",
      globals: {
        ...BROWSER_GLOBALS,
        ...scriptGlobals,
      },
    },
    rules: {
      "no-undef": "error",
    },
  };
});

export default [
  ...browserScriptConfigs,
  {
    // UMD libraries that check `typeof module === 'object' && module.exports`
    files: ["public/javascript/search.js", "public/javascript/trakt-dates.js"],
    languageOptions: {
      globals: {
        module: "readonly",
      },
    },
  },
  {
    files: ["tools/**/*.js"],
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: "commonjs",
      globals: {
        ...NODE_GLOBALS,
      },
    },
    rules: {
      "no-undef": "error",
    },
  },
];
