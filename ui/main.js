// NexusRAR — UI controller v4 (declarative action system).
//
// Architecture: a single `ctx` object is the source of truth.
// Every state change calls `setCtx({...})` which triggers a
// re-render of the action system. No if/else spaghetti — the
// action registry in actions.js decides which buttons to show.

import {
  ACTIONS,
  bindRunner,
  computeActions,
  detectFileKind,
  emptyContext,
  renderActions,
  runAction,
} from "./actions.js";

const invoke = window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);

// -----------------------------------------------------------------------
// DOM refs
// -----------------------------------------------------------------------
const $ = (id) => document.getElementById(id);

const els = {
  statusPill: $("status-pill"),
  statusText: $("status-text"),

  dropzone: $("dropzone"),
  fileInput: $("file-input"),
  dropzoneMeta: $("dropzone-meta"),

  btnBrowseFile: $("btn-browse-file"),
  btnPickFolder: $("btn-pick-folder"),

  levelSlider: $("level-slider"),
  settingMode: $("setting-mode"),
  settingHint: $("setting-hint"),
  sliderMarks: document.querySelectorAll(".slider-marks span"),

  cardFiles: $("card-files"),
  fileListBody: $("file-list-body"),
  filesHint: $("files-hint"),

  cardResult: $("card-result"),
  resultEmpty: $("result-empty"),
  resultFilled: $("result-filled"),
  resultEyebrow: $("result-eyebrow"),
  resultCheck: $("result-check"),
  barOriginal: $("bar-original"),
  barCompressed: $("bar-compressed"),
  resultOrigLabel: $("result-orig-label"),
  resultCompLabel: $("result-comp-label"),
  resultArrow: $("result-arrow"),
  resultOrig: $("result-orig"),
  resultComp: $("result-comp"),
  resultSavedVal: $("result-saved-val"),
  resultSavedPct: $("result-saved-pct"),
  resultRatio: $("result-ratio"),
  resultTime: $("result-time"),
  resultTimeLabel: $("result-time-label"),
  resultFilesLabel: $("result-files-label"),
  resultFiles: $("result-files"),

  resultActions: $("result-actions-result"),

  actionBar: document.querySelector(".action-bar"),
  actionHero: $("action-hero"),
  actionToolbar: $("action-toolbar"),
  actionMeta: $("action-meta"),
  actionStatus: $("action-status"),

  cardLogs: $("card-logs"),
  console: $("console"),
  logsHint: $("logs-hint"),
};

// -----------------------------------------------------------------------
// ActionContext — the single source of truth for UI state
// -----------------------------------------------------------------------

let ctx = emptyContext();

// Most recent file/folder bytes kept outside ctx because they're
// large blobs — only the metadata lives in ctx.
let lastFileBytes = null; // Uint8Array
let lastArchiveBytes = null; // Uint8Array (for saveArchive action)
let lastExtractedPath = null;

/**
 * Merge patch into ctx, then re-render every dynamic UI region
 * that depends on it. This is the ONLY function that mutates ctx
 * — every other place goes through here.
 */
function setCtx(patch) {
  ctx = { ...ctx, ...patch };
  renderUI();
}

/** Re-render the dynamic UI regions from the current ctx. */
function renderUI() {
  // Action system: the toolbar + hero button.
  const actions = computeActions(ctx);
  renderActions(actions, els.actionToolbar, els.actionHero);
  // Sync the "enabled" state of each rendered button from the
  // latest ctx (visibility was applied by computeActions; enabled
  // is a per-render check).
  syncEnabledState(actions);

  // Result dashboard (uses ctx to decide which layout to show).
  if (ctx.selection === null && ctx.lastResult === null) {
    showResultEmpty();
  } else {
    if (ctx.lastResult) {
      if (ctx.lastResult.kind === "compress") showCompressResult(ctx.lastResult);
      else if (ctx.lastResult.kind === "extract") showExtractResult(ctx.lastResult);
    } else {
      showSelectionPreview();
    }
  }

  // Drop zone meta + action bar meta.
  updateMeta();

  // File list (if we have a result with entries).
  if (ctx.lastResult?.entries) {
    renderFileList(ctx.lastResult, ctx.lastResult.kind === "compress" /* isLive */);
    els.cardFiles.hidden = false;
  } else if (ctx.selection?.manifest?.entries) {
    renderFileList(ctx.selection.manifest, false /* isLive */);
    els.cardFiles.hidden = false;
  } else {
    els.cardFiles.hidden = true;
  }
}

function syncEnabledState(actions) {
  // For each rendered action button, sync its enabled/disabled
  // class from the current ctx.
  for (const el of document.querySelectorAll("[data-action-id]")) {
    const a = ACTIONS[el.dataset.actionId];
    if (!a) continue;
    const enabled = a.enabled(ctx);
    el.classList.toggle("action-disabled", !enabled);
    el.disabled = !enabled;
  }
}

function updateMeta() {
  if (ctx.selection) {
    const s = ctx.selection;
    if (s.kind === "archive") {
      const color = "var(--magenta)";
      els.dropzoneMeta.textContent = `${s.name} · ${fmtBytes(s.size)} · nxar archive detected`;
      els.actionMeta.innerHTML = `<strong>${s.name}</strong> · ${fmtBytes(s.size)} · <span style="color:${color}">archive</span>`;
    } else if (s.kind === "folder") {
      els.dropzoneMeta.textContent = `${s.name} · folder`;
      els.actionMeta.innerHTML = `<strong>${s.name}</strong> · folder`;
    } else {
      els.dropzoneMeta.textContent = `${s.name} · ${fmtBytes(s.size)}`;
      els.actionMeta.innerHTML = `<strong>${s.name}</strong> · ${fmtBytes(s.size)}`;
    }
  } else {
    els.dropzoneMeta.textContent = "supports folders · .nxar archives · any file type";
    els.actionMeta.innerHTML = `<span class="muted">drop a folder, file, or .nxar archive to begin</span>`;
  }

  if (ctx.isProcessing) {
    els.actionStatus.innerHTML = `<strong style="color:var(--accent)">${ctx.state}…</strong>`;
  } else if (ctx.error) {
    els.actionStatus.innerHTML = `<strong style="color:var(--bad)">err · ${ctx.error.code}</strong>`;
  } else if (ctx.lastResult) {
    const r = ctx.lastResult;
    if (r.kind === "compress") {
      els.actionStatus.innerHTML = `<strong style="color:var(--success)">done</strong> · ${fmtRatio(r.ratio)} · ${fmtBytes(r.totalOriginal)} → ${fmtBytes(r.totalCompressed)}`;
    } else if (r.kind === "extract") {
      els.actionStatus.innerHTML = `<strong style="color:var(--success)">extracted</strong> · ${r.nFiles} files`;
    }
  } else {
    els.actionStatus.innerHTML = `<span class="muted">idle</span>`;
  }
}

// -----------------------------------------------------------------------
// Status pill + log console
// -----------------------------------------------------------------------
const STATUS_ICONS = { idle: "○", ready: "●", working: "◐", ok: "●", err: "✕" };
function setStatus(state, text) {
  els.statusPill.dataset.state = state;
  const icon = STATUS_ICONS[state] || "●";
  els.statusText.textContent = `${icon}  ${text}`;
}
const TAG_STYLES = { info: "info", ok: "ok", warn: "warn", err: "err", rx: "rx", tx: "tx" };
function log(tag, msg, html = false) {
  const ts = new Date().toLocaleTimeString("en-GB", { hour12: false }) +
    "." + String(Date.now() % 1000).padStart(3, "0");
  const line = document.createElement("div");
  line.className = "console-line";
  const tsEl = document.createElement("span");
  tsEl.className = "console-ts";
  tsEl.textContent = ts;
  const tagEl = document.createElement("span");
  tagEl.className = `console-tag ${TAG_STYLES[tag] || "info"}`;
  tagEl.textContent = tag.toUpperCase();
  const msgEl = document.createElement("span");
  msgEl.className = "console-msg";
  if (html) msgEl.innerHTML = msg;
  else msgEl.textContent = msg;
  line.append(tsEl, tagEl, msgEl);
  els.console.appendChild(line);
  els.console.scrollTop = els.console.scrollHeight;
  const count = els.console.children.length;
  els.logsHint.textContent = count === 0 ? "— empty —" : `${count} line${count === 1 ? "" : "s"}`;
  if (tag === "err" && !els.cardLogs.open) els.cardLogs.open = true;
}

// -----------------------------------------------------------------------
// Number formatting
// -----------------------------------------------------------------------
function fmtBytes(n) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(2)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(2)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}
function fmtMs(ms) {
  if (ms < 1) return `${(ms * 1000).toFixed(0)} µs`;
  if (ms < 100) return `${ms.toFixed(1)} ms`;
  if (ms < 60000) return `${Math.round(ms)} ms`;
  return `${(ms / 1000).toFixed(2)} s`;
}
function fmtRatio(r) {
  if (r <= 0) return "—";
  if (r < 1.01) return "1.00×";
  return `${r.toFixed(2)}×`;
}
function fmtPct(p) { if (p < 0) return "—"; return `${(p * 100).toFixed(1)}%`; }
function ratioClass(r) {
  if (r >= 3.0) return "ratio-great";
  if (r >= 2.0) return "ratio-good";
  if (r >= 1.0) return "ratio-meh";
  return "ratio-poor";
}
function levelName() {
  const v = Number(els.levelSlider.value);
  if (v <= 12) return "fast";
  if (v <= 62) return "balanced";
  if (v >= 88) return "maximum";
  return "custom";
}

// -----------------------------------------------------------------------
// Compression level slider (semantic)
// -----------------------------------------------------------------------
const LEVEL_PRESETS = [
  { value: 0, name: "fast", hint: "lazy LZ77 + 5-stream rANS · fastest, ~3× typical" },
  { value: 50, name: "balanced", hint: "lazy + 4-stream rANS · mid speed, ~3.5× ratio" },
  { value: 100, name: "maximum", hint: "optimal DP + 4-stream · 8.8× slower, ~0% gain (experimental)" },
];
function applyLevel(value) {
  const v = Number(value);
  let preset = LEVEL_PRESETS[0];
  let minDist = Infinity;
  for (const p of LEVEL_PRESETS) {
    const d = Math.abs(p.value - v);
    if (d < minDist) { minDist = d; preset = p; }
  }
  if (minDist <= 8) {
    els.settingMode.textContent = preset.name;
    els.settingHint.textContent = preset.hint;
    els.levelSlider.value = preset.value;
  } else {
    els.settingMode.textContent = "custom";
    els.settingHint.textContent = `slider at ${v}% · between presets`;
  }
  for (const m of els.sliderMarks) {
    m.classList.toggle("active", Math.abs(Number(m.dataset.mark) - v) <= 12);
  }
}
els.levelSlider.addEventListener("input", (e) => applyLevel(e.target.value));
applyLevel(els.levelSlider.value);

// -----------------------------------------------------------------------
// Drop zone
// -----------------------------------------------------------------------
function openFilePicker() {
  try { els.fileInput.click(); } catch (e) { log("err", `file picker failed: ${e}`); }
}
els.dropzone.addEventListener("click", openFilePicker);
els.dropzone.addEventListener("keydown", (e) => {
  if (e.key === "Enter" || e.key === " ") { e.preventDefault(); openFilePicker(); }
});
els.fileInput.addEventListener("change", () => {
  if (els.fileInput.files.length === 0) return;
  loadFile(els.fileInput.files[0]);
});
els.btnBrowseFile.addEventListener("click", (e) => { e.stopPropagation(); openFilePicker(); });
els.btnPickFolder.addEventListener("click", (e) => { e.stopPropagation(); pickAndCompressFolder(); });

["dragenter", "dragover"].forEach((ev) =>
  els.dropzone.addEventListener(ev, (e) => {
    e.preventDefault();
    e.stopPropagation();
    els.dropzone.classList.add("dragover");
  }));
["dragleave", "drop"].forEach((ev) =>
  els.dropzone.addEventListener(ev, (e) => {
    e.preventDefault();
    e.stopPropagation();
    els.dropzone.classList.remove("dragover");
  }));

// -----------------------------------------------------------------------
// Load a file (auto-detect kind)
// -----------------------------------------------------------------------
async function loadFile(file) {
  log("rx", `loaded ${file.name} (${fmtBytes(file.size)})`);
  setStatus("working", "loading…");
  const buf = await file.arrayBuffer();
  const bytes = new Uint8Array(buf);
  const kind = detectFileKind(bytes, file.name);

  if (kind === "archive") {
    lastFileBytes = bytes;
    // Peek the archive (async) — updates the file list preview.
    setCtx({
      state: "idle",
      selection: { kind: "archive", name: file.name, size: file.size, path: null, bytes, manifest: null },
      lastResult: null,
      isProcessing: true,
      error: null,
      extractedPath: null,
    });
    setStatus("working", "reading archive…");
    try {
      const result = await invoke("peek_archive_cmd", { archive: Array.from(bytes) });
      // Promote the peeked manifest into the selection.
      setCtx({
        selection: {
          kind: "archive",
          name: file.name,
          size: file.size,
          path: null,
          bytes,
          manifest: result,
        },
        isProcessing: false,
      });
      log("ok", `archive contents: <strong>${result.n_files}</strong> files · ${fmtRatio(result.aggregate_ratio)} avg`, true);
      setStatus("ready", "ready to extract");
      log("ok", "press EXTRACT to choose an output folder");
    } catch (e) {
      console.error("[nexus] peek failed:", e);
      log("err", `peek_archive failed: <strong>${e}</strong>`);
      setCtx({ isProcessing: false, error: { code: "archive.malformed", message: String(e) } });
      setStatus("err", "peek failed");
    }
  } else {
    lastFileBytes = bytes;
    setCtx({
      state: "idle",
      selection: { kind: "file", name: file.name, size: file.size, path: null, bytes, manifest: null },
      lastResult: null,
      isProcessing: false,
      error: null,
      extractedPath: null,
    });
    setStatus("ready", "ready");
    log("ok", "file ready · press COMPRESS to run the engine");
  }
}

// -----------------------------------------------------------------------
// Result dashboard
// -----------------------------------------------------------------------
function showResultEmpty() {
  els.resultEmpty.hidden = false;
  els.resultFilled.hidden = true;
  els.resultActions.hidden = true;
}
function showSelectionPreview() {
  els.resultEmpty.hidden = true;
  els.resultFilled.hidden = false;
  els.resultEyebrow.textContent = "ready to compress";
  els.resultCheck.textContent = "▸";
  els.resultCheck.style.color = "var(--accent)";
  els.resultOrigLabel.textContent = "size";
  els.resultCompLabel.textContent = "—";
  els.resultArrow.textContent = "·";
  els.resultOrig.textContent = ctx.selection ? fmtBytes(ctx.selection.size) : "—";
  els.resultComp.textContent = "—";
  els.resultSavedVal.textContent = "—";
  els.resultSavedPct.textContent = "—";
  els.resultRatio.textContent = "—";
  els.resultTime.textContent = "—";
  els.resultTimeLabel.textContent = "time";
  els.resultFiles.textContent = "1";
  els.resultFilesLabel.textContent = "files";
  els.barOriginal.style.width = "100%";
  els.barCompressed.style.width = "100%";
  els.resultActions.hidden = true;
}
function showCompressResult(r) {
  els.resultEmpty.hidden = true;
  els.resultFilled.hidden = false;
  els.resultEyebrow.textContent = "compression complete";
  els.resultCheck.textContent = "✓";
  els.resultCheck.style.color = "var(--success)";
  els.resultOrigLabel.textContent = "original";
  els.resultCompLabel.textContent = "compressed";
  els.resultArrow.textContent = "→";
  els.resultOrig.textContent = fmtBytes(r.totalOriginal);
  els.resultComp.textContent = fmtBytes(r.totalCompressed);
  els.resultRatio.textContent = fmtRatio(r.ratio);
  els.resultTimeLabel.textContent = "time";
  els.resultTime.textContent = fmtMs(r.timeMs);
  els.resultFilesLabel.textContent = "files";
  els.resultFiles.textContent = String(r.nFiles);
  const saved = Math.max(0, r.totalOriginal - r.totalCompressed);
  const savedPct = r.totalOriginal > 0 ? saved / r.totalOriginal : 0;
  els.resultSavedVal.textContent = fmtBytes(saved);
  els.resultSavedPct.textContent = fmtPct(savedPct);
  const widthPct = r.totalOriginal > 0 ? Math.max(2, (r.totalCompressed / r.totalOriginal) * 100) : 0;
  requestAnimationFrame(() => { els.barCompressed.style.width = `${widthPct}%`; });
  els.barCompressed.style.background = "";
  els.resultActions.hidden = false;
  // Render the "in-card" actions: saveArchive + selfTest + reset.
  // These are duplicates of the action-bar buttons for the
  // user-friendly "result first" flow. Use the action system.
  renderActions(
    [ACTIONS.saveArchive, ACTIONS.reset].filter((a) => a.visible(ctx)),
    els.resultActions,
    null,
  );
}
function showExtractResult(r) {
  els.resultEmpty.hidden = true;
  els.resultFilled.hidden = false;
  els.resultEyebrow.textContent = "extraction complete";
  els.resultCheck.textContent = "✓";
  els.resultCheck.style.color = "var(--success)";
  els.resultOrigLabel.textContent = "files";
  els.resultCompLabel.textContent = "recovered";
  els.resultArrow.textContent = "→";
  els.resultOrig.textContent = String(r.nFiles);
  els.resultComp.textContent = fmtBytes(r.totalOriginal);
  els.resultRatio.textContent = "100%";
  els.resultTimeLabel.textContent = "time";
  els.resultTime.textContent = fmtMs(r.timeMs);
  els.resultFilesLabel.textContent = "out dir";
  els.resultFiles.textContent = (r.path || "").split("/").pop() || (r.path || "");
  els.resultSavedVal.textContent = `${r.nFiles} files`;
  els.resultSavedPct.textContent = "recovered";
  els.barOriginal.style.width = "100%";
  els.barCompressed.style.width = "100%";
  els.barCompressed.style.background = "var(--magenta)";
  els.resultActions.hidden = false;
  renderActions(
    [ACTIONS.openExtracted, ACTIONS.reset].filter((a) => a.visible(ctx)),
    els.resultActions,
    null,
  );
}

// -----------------------------------------------------------------------
// File list
// -----------------------------------------------------------------------
function renderFileList(result, isLive) {
  if (!result || !result.entries || result.entries.length === 0) {
    els.cardFiles.hidden = true;
    return;
  }
  els.cardFiles.hidden = false;
  const hintParts = [`${result.n_files} files`];
  if (result.aggregate_ratio) hintParts.push(`avg ${fmtRatio(result.aggregate_ratio)}`);
  if (!isLive) hintParts.push("archive preview");
  els.filesHint.textContent = hintParts.join(" · ");
  const entries = [...result.entries].sort((a, b) => b.original_size - a.original_size);
  els.fileListBody.innerHTML = "";
  for (const e of entries) {
    const row = document.createElement("div");
    row.className = "file-list-row";
    const ratio = e.compressed_size > 0 ? e.original_size / e.compressed_size : 0;
    const cls = ratioClass(ratio);
    const name = document.createElement("span");
    name.className = "file-name";
    name.textContent = e.path;
    name.title = e.path;
    const orig = document.createElement("span");
    orig.className = "file-list-size";
    orig.textContent = fmtBytes(e.original_size);
    const comp = document.createElement("span");
    comp.className = "file-list-size";
    comp.textContent = fmtBytes(e.compressed_size);
    const rat = document.createElement("span");
    rat.className = `file-list-ratio ${cls}`;
    rat.textContent = fmtRatio(ratio);
    row.append(name, orig, comp, rat);
    els.fileListBody.appendChild(row);
  }
}

// -----------------------------------------------------------------------
// Action runners — wire each action to its handler
// -----------------------------------------------------------------------

bindRunner("compress", async () => {
  if (!ctx.selection) return;
  if (ctx.selection.kind === "folder") {
    await compressFolderAction();
  } else if (ctx.selection.kind === "file") {
    await compressFileAction();
  }
});

bindRunner("extract", async () => {
  if (ctx.selection?.kind === "archive") {
    await extractArchiveAction();
  }
});

bindRunner("saveArchive", async () => {
  if (ctx.lastResult?.kind === "compress" && lastArchiveBytes) {
    const name = (ctx.lastResult.path || "folder").split("/").pop() || "folder";
    downloadBlob(lastArchiveBytes, name + ".nxar");
  } else {
    log("warn", "no archive to save — compress something first");
  }
});

bindRunner("openExtracted", async () => {
  if (!ctx.extractedPath) { log("warn", "no extracted folder"); return; }
  try {
    await invoke("open_path_cmd", { path: ctx.extractedPath });
  } catch (e) {
    log("err", `open_path failed: <strong>${e}</strong>`);
  }
});

bindRunner("selfTest", async () => {
  setStatus("working", "self-test…");
  try {
    const res = await invoke("self_test_cmd");
    if (res.roundtrip_ok) {
      log("ok", `self-test PASS · ratio ${fmtRatio(res.ratio)} · compress ${fmtMs(res.compress_time_ms)} · decompress ${fmtMs(res.decompress_time_ms)}`);
      setStatus("ok", "self-test pass");
    } else {
      log("err", "self-test FAIL · roundtrip mismatch");
      setStatus("err", "self-test fail");
    }
  } catch (e) {
    log("err", `self-test failed: ${e}`);
    setStatus("err", "self-test fail");
  }
});

bindRunner("reset", () => {
  lastFileBytes = null;
  lastArchiveBytes = null;
  lastExtractedPath = null;
  setCtx(emptyContext());
  setStatus("ready", "ready");
  log("info", "ready for the next one");
});

// Listen for action events dispatched by the action system.
document.addEventListener("nexus:action", (e) => {
  const id = e.detail.id;
  runAction(id, ctx);
});

// -----------------------------------------------------------------------
// Action implementations
// -----------------------------------------------------------------------

async function compressFileAction() {
  if (!ctx.selection || ctx.selection.kind !== "file") return;
  setCtx({ state: "compressing", isProcessing: true });
  els.dropzone.classList.add("processing");
  const t0 = performance.now();
  try {
    const res = await invoke("compress_bytes_cmd", { input: Array.from(lastFileBytes) });
    const wall = performance.now() - t0;
    log("ok", `compressed in <strong>${fmtMs(res.compress_time_ms)}</strong> · ratio <strong>${fmtRatio(res.ratio)}</strong> · <strong>${fmtBytes(res.original_size)}</strong> → <strong>${fmtBytes(res.compressed_size)}</strong>`, true);
    log("info", `level=${levelName()} · wall ${fmtMs(wall)} · IPC ${fmtMs(Math.max(0, wall - res.compress_time_ms))}`);
    lastArchiveBytes = new Uint8Array(res.compressed);
    setCtx({
      state: "success",
      isProcessing: false,
      lastResult: {
        kind: "compress",
        nFiles: 1,
        totalOriginal: res.original_size,
        totalCompressed: res.compressed_size,
        ratio: res.ratio,
        timeMs: res.compress_time_ms,
        path: null,
        entries: [{
          path: ctx.selection.name,
          original_size: res.original_size,
          compressed_size: res.compressed_size,
          ratio: res.ratio,
        }],
      },
    });
    setStatus("ok", "compressed");
  } catch (e) {
    console.error("[nexus] compress failed:", e);
    log("err", `compress failed: <strong>${e}</strong>`);
    setCtx({ state: "error", isProcessing: false, error: { code: "compress.failed", message: String(e) } });
    setStatus("err", "failed");
  } finally {
    els.dropzone.classList.remove("processing");
  }
}

async function compressFolderAction() {
  if (!ctx.selection || ctx.selection.kind !== "folder") return;
  // The folder path is not in the selection (it comes from the
  // OS picker). We need to pick the folder.
  await pickAndCompressFolder();
}

async function pickAndCompressFolder() {
  setStatus("working", "picking folder…");
  let picked;
  try {
    picked = await invoke("pick_directory_cmd");
  } catch (e) {
    log("err", `folder picker failed: <strong>${e}</strong>`);
    return;
  }
  if (!picked) { log("info", "folder picker cancelled"); setStatus("ready", "ready"); return; }
  log("rx", `picked: <strong>${picked}</strong>`);
  await compressFolderAt(picked);
}

async function compressFolderAt(path) {
  setCtx({ state: "compressing", isProcessing: true });
  els.dropzone.classList.add("processing");
  const t0 = performance.now();
  try {
    const result = await invoke("compress_directory_cmd", { inputDir: path, level: levelName() });
    const [dirResult, archive] = result;
    const wall = performance.now() - t0;
    log("ok", `compressed <strong>${dirResult.n_files}</strong> files in <strong>${fmtMs(dirResult.total_time_ms)}</strong> · ratio <strong>${fmtRatio(dirResult.aggregate_ratio)}</strong> · <strong>${fmtBytes(dirResult.total_original_size)}</strong> → <strong>${fmtBytes(dirResult.total_compressed_size)}</strong>`, true);
    log("info", `level=${levelName()} · wall ${fmtMs(wall)} · IPC ${fmtMs(Math.max(0, wall - dirResult.total_time_ms))}`);
    lastArchiveBytes = new Uint8Array(archive);
    setCtx({
      state: "success",
      isProcessing: false,
      selection: { kind: "folder", name: path.split("/").pop() || "folder", size: dirResult.total_original_size, path, bytes: null, manifest: dirResult },
      lastResult: {
        kind: "compress",
        nFiles: dirResult.n_files,
        totalOriginal: dirResult.total_original_size,
        totalCompressed: dirResult.total_compressed_size,
        ratio: dirResult.aggregate_ratio,
        timeMs: dirResult.total_time_ms,
        path,
        entries: dirResult.entries,
      },
    });
    setStatus("ok", "compressed");
  } catch (e) {
    console.error("[nexus] folder compress failed:", e);
    log("err", `folder compress failed: <strong>${e}</strong>`);
    setCtx({ state: "error", isProcessing: false, error: { code: "folder.failed", message: String(e) } });
    setStatus("err", "folder failed");
  } finally {
    els.dropzone.classList.remove("processing");
  }
}

async function extractArchiveAction() {
  if (!ctx.selection || ctx.selection.kind !== "archive") return;
  setStatus("working", "picking output folder…");
  let outDir;
  try {
    outDir = await invoke("pick_directory_cmd");
  } catch (e) {
    log("err", `output folder picker failed: <strong>${e}</strong>`);
    return;
  }
  if (!outDir) { log("info", "extract cancelled"); setStatus("ready", "ready"); return; }
  log("rx", `output folder: <strong>${outDir}</strong>`);
  const baseName = (ctx.selection.name || "archive").replace(/\.nx[ar]?$/i, "");
  const target = `${outDir}/${baseName}.extracted`;
  setCtx({ state: "extracting", isProcessing: true });
  els.dropzone.classList.add("processing");
  const t0 = performance.now();
  try {
    const result = await invoke("decompress_directory_cmd", {
      archive: Array.from(lastFileBytes),
      outputDir: target,
    });
    const wall = performance.now() - t0;
    lastExtractedPath = target;
    log("ok", `extracted <strong>${result.n_files}</strong> files in <strong>${fmtMs(result.total_time_ms)}</strong> → <strong>${target}</strong>`, true);
    log("info", `wall ${fmtMs(wall)} · engine ${fmtMs(result.total_time_ms)}`);
    setCtx({
      state: "success",
      isProcessing: false,
      extractedPath: target,
      lastResult: {
        kind: "extract",
        nFiles: result.n_files,
        totalOriginal: result.total_original_size,
        totalCompressed: 0,
        ratio: 0,
        timeMs: result.total_time_ms,
        path: target,
        entries: result.entries,
      },
    });
    setStatus("ok", "extracted");
  } catch (e) {
    console.error("[nexus] extract failed:", e);
    log("err", `extract failed: <strong>${e}</strong>`);
    setCtx({ state: "error", isProcessing: false, error: { code: "extract.failed", message: String(e) } });
    setStatus("err", "extract failed");
  } finally {
    els.dropzone.classList.remove("processing");
  }
}

// -----------------------------------------------------------------------
// Utilities
// -----------------------------------------------------------------------
function downloadBlob(bytes, name) {
  const blob = new Blob([bytes], { type: "application/octet-stream" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  setTimeout(() => URL.revokeObjectURL(url), 5000);
  log("ok", `saved <strong>${name}</strong> · <strong>${fmtBytes(bytes.length)}</strong>`, true);
  log("info", "landed in your browser's Downloads folder");
}

// -----------------------------------------------------------------------
// Boot
// -----------------------------------------------------------------------
async function loadEngineInfo() {
  try {
    const info = await invoke("engine_info_cmd");
    log("info", `engine v${info.version} · format v${info.format_version} · ${info.dict_entries} dict entries`);
    setStatus("ready", "ready");
  } catch (e) {
    log("err", `engine_info failed: ${e}`);
    setStatus("err", "engine offline");
  }
}

window.addEventListener("DOMContentLoaded", () => {
  log("info", "NexusRAR booting…");
  renderUI();
  loadEngineInfo();
});
