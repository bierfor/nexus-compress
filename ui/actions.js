// NexusRAR — Action system.
//
// Declarative UI actions: each action is a data object that
// describes how it should look (label, icon, variant) and how it
// should behave (visible/enabled/run). The render layer reads
// `computeActions(ctx)` and diffs the result against the current
// DOM, so buttons animate smoothly between contexts (no jumpy
// layout).
//
// ## Adding a new action
//
// 1. Add a new entry to `ACTIONS` below with a unique `id`.
// 2. Implement `visible(ctx)` (when does the action show?),
//    `enabled(ctx)` (when is it clickable?), and `run(ctx)` (what
//    does it do?).
// 3. Add a CSS rule in `styles.css` if you introduce a new
//    `variant`.
//
// The action appears in the toolbar (and possibly the big hero
// button) automatically — no other wiring needed.

// -----------------------------------------------------------------------
// File type detection
// -----------------------------------------------------------------------

const NXAR_MAGIC = [0x4e, 0x58, 0x41, 0x52]; // "NXAR"

const ARCHIVE_EXTS = [
  "nxar", "nxr", "zip", "rar", "tar", "tar.gz", "tgz", "tbz2", "7z",
  "gz", "bz2", "xz",
];

function isNxarMagic(bytes) {
  return bytes.length >= 4 &&
    bytes[0] === NXAR_MAGIC[0] && bytes[1] === NXAR_MAGIC[1] &&
    bytes[2] === NXAR_MAGIC[2] && bytes[3] === NXAR_MAGIC[3];
}

/// Detect the "kind" of a file for action dispatch.
/// Returns: 'archive' | 'file' | 'unknown'
/// Cost: O(1) — reads at most 4 bytes for magic check.
export function detectFileKind(bytes, name) {
  if (bytes && isNxarMagic(bytes)) return "archive";
  const ext = (name || "").toLowerCase().split(".").pop() || "";
  if (ARCHIVE_EXTS.includes(ext) || ARCHIVE_EXTS.some(e => name?.toLowerCase().endsWith("." + e))) {
    return "archive";
  }
  return "file";
}

// -----------------------------------------------------------------------
// ActionContext shape
// -----------------------------------------------------------------------

/**
 * @typedef {Object} ActionContext
 * @property {'idle'|'compressing'|'extracting'|'success'|'error'} state
 * @property {null | {kind: 'file'|'folder'|'archive', name: string,
 *   size: number, path: string|null, bytes: Uint8Array|null,
 *   manifest: object|null}} selection
 * @property {null | {kind: 'compress'|'extract', nFiles: number,
 *   totalOriginal: number, totalCompressed: number, ratio: number,
 *   timeMs: number, path: string|null}} lastResult
 * @property {boolean} isProcessing
 * @property {'fast'|'balanced'|'maximum'|'custom'} level
 * @property {null | {code: string, message: string}} error
 * @property {string|null} extractedPath
 */

/** Default context — the empty state. */
export function emptyContext() {
  return {
    state: "idle",
    selection: null,
    lastResult: null,
    isProcessing: false,
    level: "fast",
    error: null,
    extractedPath: null,
  };
}

// -----------------------------------------------------------------------
// Action registry
// -----------------------------------------------------------------------

/**
 * @typedef {Object} Action
 * @property {string} id
 * @property {string} label          short label (toolbar)
 * @property {string} heroLabel      big hero button label
 * @property {string} glyph          icon char
 * @property {'primary'|'extract'|'ghost'|'success'} variant
 * @property {(ctx: ActionContext) => boolean} visible
 * @property {(ctx: ActionContext) => boolean} enabled
 * @property {(ctx: ActionContext) => Promise<void>|void} run
 * @property {number} order          smaller = earlier in toolbar
 * @property {boolean} isHero         true if this is the big primary action
 */

/**
 * Each action is a data object. The runner (in main.js) is
 * injected at render time via `bindRunner(id, fn)`.
 *
 * The keys are the action ids; the values are the descriptors.
 * Adding a new action is a single entry — no UI plumbing needed.
 */
export const ACTIONS = {
  compress: {
    id: "compress",
    label: "compress",
    heroLabel: "compress",
    glyph: "▣",
    variant: "primary",
    order: 10,
    isHero: true,
    visible: (ctx) =>
      ctx.selection !== null &&
      (ctx.selection.kind === "file" || ctx.selection.kind === "folder" || ctx.selection.kind === "folders") &&
      !ctx.isProcessing,
    enabled: (ctx) => ctx.selection !== null && !ctx.isProcessing,
  },
  compressFolders: {
    id: "compressFolders",
    label: "compress folders…",
    heroLabel: "compress multiple folders",
    glyph: "▦",
    variant: "primary",
    order: 9,
    isHero: false,
    // Always visible: it's the entry point for picking
    // multiple folders at once, even before any selection.
    visible: () => true,
    enabled: (ctx) => !ctx.isProcessing,
  },
  extract: {
    id: "extract",
    label: "extract",
    heroLabel: "extract archive",
    glyph: "▣",
    variant: "extract",
    order: 10,
    isHero: true,
    visible: (ctx) => ctx.selection?.kind === "archive" && !ctx.isProcessing,
    enabled: (ctx) => ctx.selection?.kind === "archive" && !ctx.isProcessing,
  },
  saveArchive: {
    id: "saveArchive",
    label: "save .nxr",
    heroLabel: "save .nxr archive",
    glyph: "↓",
    variant: "primary",
    order: 20,
    visible: (ctx) => ctx.lastResult?.kind === "compress" && !ctx.isProcessing,
    enabled: (ctx) => ctx.lastResult?.kind === "compress",
  },
  openExtracted: {
    id: "openExtracted",
    label: "open in finder",
    heroLabel: "open in finder",
    glyph: "↗",
    variant: "primary",
    order: 20,
    visible: (ctx) => ctx.lastResult?.kind === "extract" && ctx.extractedPath,
    enabled: (ctx) => !!ctx.extractedPath,
  },
  selfTest: {
    id: "selfTest",
    label: "self-test",
    heroLabel: "self-test",
    glyph: "✓",
    variant: "ghost",
    order: 30,
    visible: () => true,
    enabled: (ctx) => !ctx.isProcessing,
  },
  reset: {
    id: "reset",
    label: "reset",
    heroLabel: "reset",
    glyph: "↺",
    variant: "ghost",
    order: 31,
    visible: (ctx) => ctx.selection !== null || ctx.lastResult !== null,
    enabled: () => true,
  },
};

// -----------------------------------------------------------------------
// Action runner binding
// -----------------------------------------------------------------------

const RUNNERS = new Map();

/**
 * Register the handler for an action. Called by main.js to
 * wire actions to the backend calls. Without this, the action
 * is non-functional.
 */
export function bindRunner(id, fn) {
  RUNNERS.set(id, fn);
}

/** Run an action by id. Returns a Promise that rejects on error. */
export function runAction(id, ctx) {
  const fn = RUNNERS.get(id);
  if (!fn) {
    console.warn(`[actions] no runner for action '${id}'`);
    return Promise.resolve();
  }
  return Promise.resolve(fn(ctx));
}

// -----------------------------------------------------------------------
// computeActions
// -----------------------------------------------------------------------

/**
 * Pure function: given an ActionContext, return the array of
 * visible actions in display order. Actions that are not visible
 * are excluded; actions that are not enabled are included but
 * rendered in their disabled state.
 */
export function computeActions(ctx) {
  const out = [];
  for (const id in ACTIONS) {
    const a = ACTIONS[id];
    if (a.visible(ctx)) out.push(a);
  }
  out.sort((a, b) => a.order - b.order || a.label.localeCompare(b.label));
  return out;
}

// -----------------------------------------------------------------------
// Diff-based renderer
// -----------------------------------------------------------------------

/**
 * Render actions into a container, diffing against the current
 * DOM so existing buttons animate smoothly (no remount flash).
 *
 * The first action in the array is the hero (the big primary
 * button). If `heroContainer` is provided, it's rendered there.
 * Otherwise the first action lands in the regular container.
 */
export function renderActions(actions, container, heroContainer) {
  // Hero: the first isHero action, or the first action overall.
  const hero = actions.find((a) => a.isHero) || actions[0];
  const secondary = actions.filter((a) => a !== hero);

  if (heroContainer) renderInto(hero ? [hero] : [], heroContainer, "hero");
  renderInto(secondary, container, "toolbar");
}

function renderInto(actions, container, slot) {
  // Diff: build a map of existing buttons by id.
  const existing = new Map();
  for (const el of container.querySelectorAll("[data-action-id]")) {
    existing.set(el.dataset.actionId, el);
  }

  const seen = new Set();
  for (const a of actions) {
    seen.add(a.id);
    let el = existing.get(a.id);
    if (!el) {
      el = createActionEl(a, slot);
      // Animate the entrance.
      el.classList.add("action-entering");
      container.appendChild(el);
      requestAnimationFrame(() => el.classList.remove("action-entering"));
    } else {
      updateActionEl(el, a, slot);
      // Re-append to reorder (cheap since DOM is small).
      container.appendChild(el);
    }
  }

  // Remove actions that are no longer in the list.
  for (const [id, el] of existing) {
    if (!seen.has(id)) {
      el.classList.add("action-leaving");
      el.addEventListener("transitionend", () => el.remove(), { once: true });
      // Safety net if no transition fires.
      setTimeout(() => el.parentNode === container && el.remove(), 400);
    }
  }
}

function createActionEl(action, slot) {
  const el = document.createElement("button");
  el.className = `action-btn action-slot-${slot} action-variant-${action.variant}`;
  el.dataset.actionId = action.id;
  el.type = "button";
  el.innerHTML = `
    <span class="action-glyph">${action.glyph || ""}</span>
    <span class="action-label"></span>
  `;
  updateActionEl(el, action, slot);
  el.addEventListener("click", () => {
    if (el.classList.contains("action-disabled")) return;
    el.classList.add("action-pulse");
    setTimeout(() => el.classList.remove("action-pulse"), 200);
    // The host wires the runner — fire a CustomEvent on the
    // container so main.js can dispatch.
    containerEvent(el, action.id);
  });
  return el;
}

function updateActionEl(el, action, slot) {
  el.className = `action-btn action-slot-${slot} action-variant-${action.variant}`;
  const labelEl = el.querySelector(".action-label");
  const glyphEl = el.querySelector(".action-glyph");
  const label = slot === "hero" ? action.heroLabel : action.label;
  if (labelEl.textContent !== label) labelEl.textContent = label;
  if (glyphEl.textContent !== (action.glyph || "")) {
    glyphEl.textContent = action.glyph || "";
  }
}

function containerEvent(el, actionId) {
  const ev = new CustomEvent("nexus:action", {
    bubbles: true,
    detail: { id: actionId, element: el },
  });
  el.dispatchEvent(ev);
}
