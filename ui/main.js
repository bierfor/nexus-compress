// NexusRAR — UI controller v3 (decompress-aware).
//
// Two modes:
//   1. COMPRESS (cyan): file or folder dropped → COMPRESS button
//      → result dashboard with "saved X MB (Y%)" + save .nxr
//   2. EXTRACT (magenta): .nxar file dropped → EXTRACT button
//      → result dashboard with archive contents preview
//      → folder picker → "open in finder"
//
// Mode is auto-detected by checking the NXAR magic on the first
// 4 bytes of the dropped file.

const invoke = window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);

// NXAR magic for mode detection.
const NXAR_MAGIC = [0x4e, 0x58, 0x41, 0x52]; // "NXAR"

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

  compressActions: $("compress-actions"),
  extractActions: $("extract-actions"),
  btnSaveArchive: $("btn-save-archive"),
  btnAnother: $("btn-another"),
  btnSelfTest: $("btn-self-test"),
  btnOpenExtracted: $("btn-open-extracted"),
  btnArchiveSave: $("btn-archive-save"),
  btnExtractAnother: $("btn-extract-another"),

  btnCompress: $("btn-compress"),
  actionMeta: $("action-meta"),
  actionStatus: $("action-status"),

  cardLogs: $("card-logs"),
  console: $("console"),
  logsHint: $("logs-hint"),
};

// -----------------------------------------------------------------------
// State
// -----------------------------------------------------------------------
let lastFile = null;          // { name, bytes: Uint8Array } — single file
let lastOutput = null;        // { name, bytes, isCompressed: bool }
let lastDirResult = null;     // DirectoryResult from compress_directory
let lastArchive = null;       // NXAR archive bytes
let lastDirPath = null;       // folder path of last dir compress
let lastExtractedPath = null; // folder path of last extract (for "open in finder")
let mode = "idle";            // "idle" | "compress" | "extract"

// -----------------------------------------------------------------------
// Status pill
// -----------------------------------------------------------------------
const STATUS_ICONS = {
  idle: "○", ready: "●", working: "◐", ok: "●", err: "✕",
};
function setStatus(state, text) {
  els.statusPill.dataset.state = state;
  const icon = STATUS_ICONS[state] || "●";
  els.statusText.textContent = `${icon}  ${text}`;
}

// -----------------------------------------------------------------------
// Console / logs
// -----------------------------------------------------------------------
const TAG_STYLES = {
  info: "info", ok: "ok", warn: "warn", err: "err", rx: "rx", tx: "tx",
};
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
  els.logsHint.textContent = count === 0
    ? "— empty —"
    : `${count} line${count === 1 ? "" : "s"}`;

  if (tag === "err" && !els.cardLogs.open) {
    els.cardLogs.open = true;
  }
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
function fmtPct(p) {
  if (p < 0) return "—";
  return `${(p * 100).toFixed(1)}%`;
}
function ratioClass(r) {
  if (r >= 3.0) return "ratio-great";
  if (r >= 2.0) return "ratio-good";
  if (r >= 1.0) return "ratio-meh";
  return "ratio-poor";
}

// -----------------------------------------------------------------------
// Compression level slider (semantic: 0=speed, 100=ratio)
// -----------------------------------------------------------------------
const LEVEL_PRESETS = [
  { value: 0,   name: "fast",     hint: "lazy LZ77 + 5-stream rANS · fastest, ~3× typical" },
  { value: 50,  name: "balanced", hint: "lazy + 4-stream rANS · mid speed, ~3.5× ratio" },
  { value: 100, name: "maximum",  hint: "optimal DP + 4-stream · 8.8× slower, ~0% gain (experimental)" },
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
    const markVal = Number(m.dataset.mark);
    m.classList.toggle("active", Math.abs(markVal - v) <= 12);
  }
}
els.levelSlider.addEventListener("input", (e) => applyLevel(e.target.value));
applyLevel(els.levelSlider.value);

function levelName() {
  const v = Number(els.levelSlider.value);
  if (v <= 12) return "fast";
  if (v <= 62) return "balanced";
  if (v >= 88) return "maximum";
  return "custom";
}

// -----------------------------------------------------------------------
// Mode detection + dynamic UI
// -----------------------------------------------------------------------
function isNxarMagic(bytes) {
  return bytes.length >= 4 &&
    bytes[0] === NXAR_MAGIC[0] && bytes[1] === NXAR_MAGIC[1] &&
    bytes[2] === NXAR_MAGIC[2] && bytes[3] === NXAR_MAGIC[3];
}

/// Update the big primary action button based on the current mode.
function updateCompressButton() {
  const btn = els.btnCompress;
  const label = btn.querySelector(".btn-huge-label");
  btn.classList.remove("btn-primary", "btn-extract");
  if (mode === "extract") {
    btn.classList.add("btn-extract");
    label.textContent = "extract archive";
    btn.disabled = !lastFile;
  } else if (lastDirResult) {
    btn.classList.add("btn-primary");
    label.textContent = "compress folder";
    btn.disabled = false;
  } else if (lastFile) {
    btn.classList.add("btn-primary");
    label.textContent = "compress file";
    btn.disabled = false;
  } else {
    label.textContent = "compress";
    btn.disabled = true;
  }
}

function setMode(newMode) {
  mode = newMode;
  // Mode-specific UI tweaks.
  if (mode === "extract") {
    els.dropzoneMeta.textContent =
      "ready to extract · pick an output folder below";
  } else {
    els.dropzoneMeta.textContent =
      "supports folders · .nxar archives · any file type";
  }
  updateCompressButton();
}

// -----------------------------------------------------------------------
// Drop zone
// -----------------------------------------------------------------------
function openFilePicker() {
  try {
    els.fileInput.click();
  } catch (e) {
    log("err", `file picker failed: ${e}`);
  }
}
els.dropzone.addEventListener("click", openFilePicker);
els.dropzone.addEventListener("keydown", (e) => {
  if (e.key === "Enter" || e.key === " ") {
    e.preventDefault();
    openFilePicker();
  }
});
els.fileInput.addEventListener("change", () => {
  if (els.fileInput.files.length === 0) return;
  loadFile(els.fileInput.files[0]);
});
els.btnBrowseFile.addEventListener("click", (e) => {
  e.stopPropagation();
  openFilePicker();
});
els.btnPickFolder.addEventListener("click", (e) => {
  e.stopPropagation();
  pickAndCompressFolder();
});

["dragenter", "dragover"].forEach((ev) =>
  els.dropzone.addEventListener(ev, (e) => {
    e.preventDefault();
    e.stopPropagation();
    els.dropzone.classList.add("dragover");
  }),
);
["dragleave", "drop"].forEach((ev) =>
  els.dropzone.addEventListener(ev, (e) => {
    e.preventDefault();
    e.stopPropagation();
    els.dropzone.classList.remove("dragover");
  }),
);

// -----------------------------------------------------------------------
// Load a file (with mode detection)
// -----------------------------------------------------------------------
async function loadFile(file) {
  log("rx", `loaded ${file.name} (${fmtBytes(file.size)})`);
  setStatus("working", "loading…");
  const buf = await file.arrayBuffer();
  const bytes = new Uint8Array(buf);
  lastFile = { name: file.name, bytes };
  lastOutput = null;
  lastDirResult = null;
  lastArchive = null;
  lastDirPath = null;
  lastExtractedPath = null;

  if (isNxarMagic(bytes)) {
    setMode("extract");
    els.dropzoneMeta.textContent = `${file.name} · ${fmtBytes(file.size)} · nxar archive detected`;
    els.actionMeta.innerHTML = `<strong>${file.name}</strong> · ${fmtBytes(file.size)} · <span style="color:var(--magenta)">archive</span>`;
    log("info", "NXAR magic detected — switching to extract mode");
    await peekAndShowArchive(bytes);
    setStatus("ready", "ready to extract");
    log("ok", "press EXTRACT to choose an output folder");
  } else {
    setMode("compress");
    els.dropzoneMeta.textContent = `${file.name} · ${fmtBytes(file.size)}`;
    els.actionMeta.innerHTML = `<strong>${file.name}</strong> · ${fmtBytes(file.size)}`;
    updateResultForSingleFile();
    setStatus("ready", "ready");
    log("ok", "file ready · press COMPRESS to run the engine");
  }
}

/// Peek at the .nxar manifest and populate the file list preview.
async function peekAndShowArchive(bytes) {
  try {
    const result = await invoke("peek_archive_cmd", { archive: Array.from(bytes) });
    lastDirResult = result; // reuse the same shape for the file list
    renderFileList(result);
    showPeekResult(result);
  } catch (e) {
    log("err", `peek_archive failed: <strong>${e}</strong>`);
    log("warn", "the file may be corrupted or not a valid NXAR archive");
    setStatus("err", "peek failed");
  }
}

function showPeekResult(result) {
  els.resultEmpty.hidden = true;
  els.resultFilled.hidden = false;
  els.resultEyebrow.textContent = "archive contents";
  els.resultCheck.textContent = "▸";
  els.resultCheck.style.color = "var(--magenta)";
  els.resultOrigLabel.textContent = "files";
  els.resultCompLabel.textContent = "compressed";
  els.resultArrow.textContent = "·";
  els.resultOrig.textContent = String(result.n_files);
  els.resultComp.textContent = fmtBytes(result.total_compressed_size);
  els.resultSavedVal.textContent = fmtBytes(result.total_original_size);
  els.resultSavedPct.textContent = "in archive";
  els.resultRatio.textContent = fmtRatio(result.aggregate_ratio);
  els.resultTimeLabel.textContent = "size";
  els.resultTime.textContent = fmtBytes(
    result.entries.reduce((a, e) => a + e.compressed_size, 0)
  );
  els.resultFilesLabel.textContent = "ratio";
  els.resultFiles.textContent = fmtRatio(result.aggregate_ratio);
  els.barOriginal.style.width = "100%";
  els.barCompressed.style.width = `${result.aggregate_ratio > 0 ? Math.max(2, 100 / result.aggregate_ratio) : 100}%`;
  els.compressActions.hidden = true;
  els.extractActions.hidden = false;
  els.actionStatus.innerHTML = `<strong style="color:var(--magenta)">archive</strong> · ${result.n_files} files`;
}

function updateResultForSingleFile() {
  if (!lastFile) {
    showResultEmpty();
    return;
  }
  els.resultEmpty.hidden = true;
  els.resultFilled.hidden = false;
  els.resultEyebrow.textContent = "ready to compress";
  els.resultCheck.textContent = "▸";
  els.resultCheck.style.color = "var(--accent)";
  els.resultOrigLabel.textContent = "original";
  els.resultCompLabel.textContent = "compressed";
  els.resultArrow.textContent = "→";
  els.resultOrig.textContent = fmtBytes(lastFile.bytes.length);
  els.resultComp.textContent = "—";
  els.resultSavedVal.textContent = "—";
  els.resultSavedPct.textContent = "—";
  els.resultRatio.textContent = "—";
  els.resultTimeLabel.textContent = "time";
  els.resultTime.textContent = "—";
  els.resultFilesLabel.textContent = "files";
  els.resultFiles.textContent = "1";
  els.barOriginal.style.width = "100%";
  els.barCompressed.style.width = "100%";
  els.compressActions.hidden = true;
  els.extractActions.hidden = true;
}

function showResultEmpty() {
  els.resultEmpty.hidden = false;
  els.resultFilled.hidden = true;
  els.compressActions.hidden = true;
  els.extractActions.hidden = true;
  els.cardFiles.hidden = true;
}

function showResult(origSize, compSize, ratio, timeMs, nFiles = 1) {
  els.resultEmpty.hidden = true;
  els.resultFilled.hidden = false;
  els.resultEyebrow.textContent = "compression complete";
  els.resultCheck.textContent = "✓";
  els.resultCheck.style.color = "var(--success)";
  els.resultOrigLabel.textContent = "original";
  els.resultCompLabel.textContent = "compressed";
  els.resultArrow.textContent = "→";
  els.resultOrig.textContent = fmtBytes(origSize);
  els.resultComp.textContent = fmtBytes(compSize);
  els.resultRatio.textContent = fmtRatio(ratio);
  els.resultTimeLabel.textContent = "time";
  els.resultTime.textContent = fmtMs(timeMs);
  els.resultFilesLabel.textContent = "files";
  els.resultFiles.textContent = String(nFiles);
  const saved = Math.max(0, origSize - compSize);
  const savedPct = origSize > 0 ? saved / origSize : 0;
  els.resultSavedVal.textContent = fmtBytes(saved);
  els.resultSavedPct.textContent = fmtPct(savedPct);
  const widthPct = origSize > 0 ? Math.max(2, (compSize / origSize) * 100) : 0;
  requestAnimationFrame(() => {
    els.barCompressed.style.width = `${widthPct}%`;
  });
  els.compressActions.hidden = false;
  els.extractActions.hidden = true;
}

function showExtractionDone(result, outputDir) {
  els.resultEmpty.hidden = true;
  els.resultFilled.hidden = false;
  els.resultEyebrow.textContent = "extraction complete";
  els.resultCheck.textContent = "✓";
  els.resultCheck.style.color = "var(--success)";
  els.resultOrigLabel.textContent = "files";
  els.resultCompLabel.textContent = "recovered";
  els.resultArrow.textContent = "→";
  els.resultOrig.textContent = String(result.n_files);
  els.resultComp.textContent = fmtBytes(result.total_original_size);
  els.resultRatio.textContent = "100%";
  els.resultTimeLabel.textContent = "time";
  els.resultTime.textContent = fmtMs(result.total_time_ms);
  els.resultFilesLabel.textContent = "out dir";
  els.resultFiles.textContent = outputDir.split("/").pop() || outputDir;
  els.resultSavedVal.textContent = `${result.n_files} files`;
  els.resultSavedPct.textContent = "recovered";
  els.barOriginal.style.width = "100%";
  els.barCompressed.style.width = "100%";
  els.barCompressed.style.background = "var(--magenta)";
  els.compressActions.hidden = true;
  els.extractActions.hidden = false;
  els.actionStatus.innerHTML = `<strong style="color:var(--success)">extracted</strong> · ${result.n_files} files`;
}

// -----------------------------------------------------------------------
// File list with color-coded ratios
// -----------------------------------------------------------------------
function renderFileList(result) {
  if (!result || !result.entries || result.entries.length === 0) {
    els.cardFiles.hidden = true;
    return;
  }
  els.cardFiles.hidden = false;
  const isPeek = result.total_time_ms === 0.0 && !result.root.startsWith("/");
  if (isPeek) {
    els.filesHint.textContent = `${result.n_files} files · ${fmtRatio(result.aggregate_ratio)} avg · archive preview`;
  } else {
    els.filesHint.textContent = `${result.n_files} files · avg ${fmtRatio(result.aggregate_ratio)}`;
  }
  const entries = [...result.entries].sort(
    (a, b) => b.original_size - a.original_size,
  );
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
// Big primary button — dispatch by mode
// -----------------------------------------------------------------------
els.btnCompress.addEventListener("click", async () => {
  if (mode === "extract") {
    await extractArchive();
  } else if (lastDirResult && lastArchive) {
    // Re-show the last folder result without recompressing.
    showResult(
      lastDirResult.total_original_size,
      lastDirResult.total_compressed_size,
      lastDirResult.aggregate_ratio,
      lastDirResult.total_time_ms,
      lastDirResult.n_files,
    );
  } else if (lastFile) {
    await compressSingleFile();
  }
});

// -----------------------------------------------------------------------
// Compress single file
// -----------------------------------------------------------------------
async function compressSingleFile() {
  if (!lastFile) return;
  setStatus("working", "compressing…");
  els.dropzone.classList.add("processing");
  const level = levelName();
  const t0 = performance.now();
  try {
    const input = Array.from(lastFile.bytes);
    const res = await invoke("compress_bytes_cmd", { input });
    const wall = performance.now() - t0;
    showResult(res.original_size, res.compressed_size, res.ratio, res.compress_time_ms, 1);
    log(
      "ok",
      `compressed in <strong>${fmtMs(res.compress_time_ms)}</strong> · ` +
        `ratio <strong>${fmtRatio(res.ratio)}</strong> · ` +
        `<strong>${fmtBytes(res.original_size)}</strong> → <strong>${fmtBytes(res.compressed_size)}</strong>`,
      true,
    );
    log("info", `level=${level} · wall ${fmtMs(wall)} · IPC ${fmtMs(Math.max(0, wall - res.compress_time_ms))}`);
    lastOutput = {
      name: lastFile.name + ".nxr",
      bytes: new Uint8Array(res.compressed),
      isCompressed: true,
    };
    els.actionStatus.innerHTML = `<strong style="color:var(--success)">done</strong> · ${fmtRatio(res.ratio)}`;
    setStatus("ok", "compressed");
  } catch (e) {
    console.error("[nexus] compress_bytes_cmd failed:", e);
    log("err", `compress failed: <strong>${e}</strong>`);
    setStatus("err", "failed");
  } finally {
    els.dropzone.classList.remove("processing");
  }
}

// -----------------------------------------------------------------------
// Compress folder
// -----------------------------------------------------------------------
async function pickAndCompressFolder() {
  log("tx", "open folder picker…");
  setStatus("working", "picking folder…");
  let picked;
  try {
    picked = await invoke("pick_directory_cmd");
  } catch (e) {
    log("err", `folder picker failed: <strong>${e}</strong>`);
    setStatus("err", "picker failed");
    return;
  }
  if (!picked) { log("info", "folder picker cancelled"); setStatus("ready", "ready"); return; }
  log("rx", `picked: <strong>${picked}</strong>`);
  setStatus("working", "compressing folder…");
  els.dropzone.classList.add("processing");
  hideResultDashboard();
  els.cardFiles.hidden = true;
  const level = levelName();
  const t0 = performance.now();
  try {
    const result = await invoke("compress_directory_cmd", { inputDir: picked, level });
    const [dirResult, archive] = result;
    const wall = performance.now() - t0;
    showResult(
      dirResult.total_original_size, dirResult.total_compressed_size,
      dirResult.aggregate_ratio, dirResult.total_time_ms, dirResult.n_files,
    );
    log(
      "ok",
      `compressed <strong>${dirResult.n_files}</strong> files in ` +
        `<strong>${fmtMs(dirResult.total_time_ms)}</strong> · ` +
        `ratio <strong>${fmtRatio(dirResult.aggregate_ratio)}</strong> · ` +
        `<strong>${fmtBytes(dirResult.total_original_size)}</strong> → ` +
        `<strong>${fmtBytes(dirResult.total_compressed_size)}</strong>`,
      true,
    );
    log("info", `level=${level} · wall ${fmtMs(wall)} · IPC ${fmtMs(Math.max(0, wall - dirResult.total_time_ms))}`);
    lastDirResult = dirResult;
    lastArchive = new Uint8Array(archive);
    lastDirPath = picked;
    renderFileList(dirResult);
    els.actionMeta.innerHTML = `<strong>${picked.split("/").pop()}</strong> · ${dirResult.n_files} files · ${fmtBytes(dirResult.total_original_size)}`;
    els.actionStatus.innerHTML = `<strong style="color:var(--success)">done</strong> · ${fmtRatio(dirResult.aggregate_ratio)}`;
    setStatus("ok", "compressed");
  } catch (e) {
    console.error("[nexus] compress_directory_cmd failed:", e);
    log("err", `folder compress failed: <strong>${e}</strong>`);
    setStatus("err", "folder failed");
  } finally {
    els.dropzone.classList.remove("processing");
  }
}

function hideResultDashboard() {
  els.resultEmpty.hidden = false;
  els.resultFilled.hidden = true;
  els.compressActions.hidden = true;
  els.extractActions.hidden = true;
}

// -----------------------------------------------------------------------
// Extract archive (the new decompress flow)
// -----------------------------------------------------------------------
async function extractArchive() {
  if (!lastFile || mode !== "extract") return;
  // 1. Ask the user where to extract.
  setStatus("working", "picking output folder…");
  let outDir;
  try {
    outDir = await invoke("pick_directory_cmd");
  } catch (e) {
    log("err", `output folder picker failed: <strong>${e}</strong>`);
    setStatus("err", "picker failed");
    return;
  }
  if (!outDir) { log("info", "extract cancelled (no output folder)"); setStatus("ready", "ready"); return; }
  log("rx", `output folder: <strong>${outDir}</strong>`);

  // 2. Build the target subdir name.
  const baseName = (lastFile.name || "archive").replace(/\.nx[ar]?$/i, "");
  const target = `${outDir}/${baseName}.extracted`;

  // 3. Extract.
  setStatus("working", "extracting…");
  els.dropzone.classList.add("processing");
  const t0 = performance.now();
  try {
    const input = Array.from(lastFile.bytes);
    const result = await invoke("decompress_directory_cmd", {
      archive: input,
      outputDir: target,
    });
    const wall = performance.now() - t0;
    lastExtractedPath = target;
    lastDirResult = result;
    showExtractionDone(result, target);
    log(
      "ok",
      `extracted <strong>${result.n_files}</strong> files in ` +
        `<strong>${fmtMs(result.total_time_ms)}</strong> → <strong>${target}</strong>`,
      true,
    );
    log("info", `wall ${fmtMs(wall)} · engine ${fmtMs(result.total_time_ms)}`);
    setStatus("ok", "extracted");
  } catch (e) {
    console.error("[nexus] extract failed:", e);
    log("err", `extract failed: <strong>${e}</strong>`);
    if (typeof e === "string") {
      if (e.includes("malformed")) log("warn", "the file isn't a valid NXAR archive");
      else if (e.includes("size_mismatch")) log("warn", "recovered size doesn't match header — archive may be corrupted");
      else if (e.includes("io")) log("warn", "filesystem error — check the output folder permissions");
    }
    setStatus("err", "extract failed");
  } finally {
    els.dropzone.classList.remove("processing");
  }
}

// -----------------------------------------------------------------------
// Result-action buttons
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

els.btnSaveArchive.addEventListener("click", () => {
  if (lastArchive && lastDirPath) {
    downloadBlob(lastArchive, (lastDirPath.split("/").pop() || "folder") + ".nxar");
  } else if (lastOutput && lastOutput.isCompressed) {
    downloadBlob(lastOutput.bytes, lastOutput.name);
  } else {
    log("warn", "no archive to save — compress something first");
  }
});

els.btnArchiveSave.addEventListener("click", () => {
  if (lastFile && mode === "extract") {
    downloadBlob(lastFile.bytes, lastFile.name);
  } else {
    log("warn", "no archive loaded");
  }
});

els.btnOpenExtracted.addEventListener("click", async () => {
  if (!lastExtractedPath) { log("warn", "no extracted folder"); return; }
  log("tx", `open in finder: ${lastExtractedPath}`);
  try {
    await invoke("open_path_cmd", { path: lastExtractedPath });
  } catch (e) {
    log("err", `open_path failed: <strong>${e}</strong>`);
  }
});

els.btnAnother.addEventListener("click", resetState);
els.btnExtractAnother.addEventListener("click", resetState);
els.btnSelfTest.addEventListener("click", runSelfTest);

function resetState() {
  lastFile = null;
  lastOutput = null;
  lastDirResult = null;
  lastArchive = null;
  lastDirPath = null;
  lastExtractedPath = null;
  setMode("idle");
  showResultEmpty();
  els.cardFiles.hidden = true;
  els.actionMeta.innerHTML = `<span class="muted">drop a folder, file, or .nxar archive to begin</span>`;
  els.actionStatus.innerHTML = `<span class="muted">idle</span>`;
  els.dropzoneMeta.textContent = "supports folders · .nxar archives · any file type";
  setStatus("ready", "ready");
  log("info", "ready for the next one");
}

async function runSelfTest() {
  log("tx", "self-test · 8KB canned sample");
  setStatus("working", "self-test…");
  try {
    const res = await invoke("self_test_cmd");
    if (res.roundtrip_ok) {
      log(
        "ok",
        `self-test PASS · ratio ${fmtRatio(res.ratio)} · ` +
          `compress ${fmtMs(res.compress_time_ms)} · ` +
          `decompress ${fmtMs(res.decompress_time_ms)}`,
      );
      setStatus("ok", "self-test pass");
    } else {
      log("err", "self-test FAIL · roundtrip mismatch");
      setStatus("err", "self-test fail");
    }
  } catch (e) {
    log("err", `self-test failed: ${e}`);
    setStatus("err", "self-test fail");
  }
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
  setMode("idle");
  loadEngineInfo();
});
