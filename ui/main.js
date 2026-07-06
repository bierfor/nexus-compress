// NexusRAR — UI controller v2.
//
// Hierarchy:
//   1. drop zone (centre of attention)
//   2. result dashboard (big visual reward)
//   3. settings (collapsed advanced)
//   4. logs (collapsed by default)
//
// Talks to Rust via `window.__TAURI_INTERNALS__.invoke` (Tauri 2.x
// without the @tauri-apps/api npm package).

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
  barOriginal: $("bar-original"),
  barCompressed: $("bar-compressed"),
  resultOrig: $("result-orig"),
  resultComp: $("result-comp"),
  resultSavedVal: $("result-saved-val"),
  resultSavedPct: $("result-saved-pct"),
  resultRatio: $("result-ratio"),
  resultTime: $("result-time"),
  resultFiles: $("result-files"),

  btnCompress: $("btn-compress"),
  btnSaveArchive: $("btn-save-archive"),
  btnAnother: $("btn-another"),
  btnSelfTest: $("btn-self-test"),

  actionMeta: $("action-meta"),
  actionStatus: $("action-status"),

  cardLogs: $("card-logs"),
  console: $("console"),
  logsHint: $("logs-hint"),
};

// -----------------------------------------------------------------------
// State
// -----------------------------------------------------------------------
let lastFile = null; // { name, bytes: Uint8Array } — single file
let lastOutput = null; // { name, bytes, isCompressed: bool }
let lastDirResult = null; // DirectoryResult from compress_directory
let lastArchive = null; // NXAR archive bytes
let lastDirPath = null; // folder path of last dir compress

// -----------------------------------------------------------------------
// Status pill (4 states: idle, working, ok, err)
// -----------------------------------------------------------------------
const STATUS_ICONS = {
  idle: "○",
  ready: "●",
  working: "◐",
  ok: "●",
  err: "✕",
};
function setStatus(state, text) {
  els.statusPill.dataset.state = state;
  els.statusText.textContent = text;
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

  // Update the hint in the collapsed summary.
  const count = els.console.children.length;
  els.logsHint.textContent = count === 0 ? "— empty —" : `${count} line${count === 1 ? "" : "s"}`;

  // Auto-open logs on errors so the user sees what went wrong.
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

// Ratio color class
function ratioClass(r) {
  if (r >= 3.0) return "ratio-great"; // great: green
  if (r >= 2.0) return "ratio-good";  // good: cyan/blue
  if (r >= 1.0) return "ratio-meh";   // meh: yellow
  return "ratio-poor";                  // poor: red
}

// -----------------------------------------------------------------------
// Compression level slider (semantic: 0=speed, 100=ratio)
// -----------------------------------------------------------------------
const LEVEL_PRESETS = [
  { value: 0,  name: "fast",     hint: "lazy LZ77 + 5-stream rANS · fastest, ~3× typical" },
  { value: 50, name: "balanced", hint: "lazy + 4-stream rANS · mid speed, ~3.5× ratio" },
  { value: 100,name: "maximum",  hint: "optimal DP + 4-stream · 8.8× slower, ~0% gain (experimental)" },
];

function applyLevel(value) {
  // Snap to nearest preset, but allow arbitrary values for "Custom".
  const v = Number(value);
  let preset = LEVEL_PRESETS[0];
  let minDist = Infinity;
  for (const p of LEVEL_PRESETS) {
    const d = Math.abs(p.value - v);
    if (d < minDist) {
      minDist = d;
      preset = p;
    }
  }
  if (minDist <= 8) {
    // Snap to preset.
    els.settingMode.textContent = preset.name;
    els.settingHint.textContent = preset.hint;
    els.levelSlider.value = preset.value;
  } else {
    els.settingMode.textContent = "custom";
    els.settingHint.textContent = `slider at ${v}% · between presets`;
  }
  // Highlight the closest mark.
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

// OS dialog buttons inside the dropzone (the link-btns)
els.btnBrowseFile.addEventListener("click", (e) => {
  e.stopPropagation();
  openFilePicker();
});
els.btnPickFolder.addEventListener("click", (e) => {
  e.stopPropagation();
  pickAndCompressFolder();
});

// HTML5 drag-drop feedback (Tauri 2.x intercepts OS drops
// before they reach the webview, so the handlers here are
// only for the dragover visual state).
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
// Load a single file (from the file picker)
// -----------------------------------------------------------------------
async function loadFile(file) {
  log("rx", `loaded ${file.name} (${fmtBytes(file.size)})`);
  setStatus("working", "loading…");
  const buf = await file.arrayBuffer();
  lastFile = { name: file.name, bytes: new Uint8Array(buf) };
  lastOutput = null;
  lastDirResult = null;
  lastArchive = null;
  updateCompressButton();
  updateResultForSingleFile();
  els.cardFiles.hidden = true;
  els.dropzoneMeta.textContent = `${file.name} · ${fmtBytes(file.size)}`;
  els.actionMeta.innerHTML = `<strong>${file.name}</strong> · ${fmtBytes(file.size)}`;
  setStatus("ready", "ready");
  log("ok", "file ready · press COMPRESS to run the engine");
}

// -----------------------------------------------------------------------
// Update the BIG COMPRESS button label & meta
// -----------------------------------------------------------------------
function updateCompressButton() {
  const hasFile = lastFile !== null;
  const hasFolder = lastDirResult !== null;
  const ready = hasFile || hasFolder;
  els.btnCompress.disabled = !ready;
  if (hasFolder) {
    els.btnCompress.querySelector(".btn-huge-label").textContent = "compress folder";
  } else if (hasFile) {
    els.btnCompress.querySelector(".btn-huge-label").textContent = "compress file";
  } else {
    els.btnCompress.querySelector(".btn-huge-label").textContent = "compress";
  }
}

// -----------------------------------------------------------------------
// Single-file result display
// -----------------------------------------------------------------------
function updateResultForSingleFile() {
  if (!lastFile) {
    showResultEmpty();
    return;
  }
  els.resultEmpty.hidden = true;
  els.resultFilled.hidden = false;
  els.resultOrig.textContent = fmtBytes(lastFile.bytes.length);
  els.resultComp.textContent = "—";
  els.resultSavedVal.textContent = "—";
  els.resultSavedPct.textContent = "—%";
  els.resultRatio.textContent = "—";
  els.resultTime.textContent = "—";
  els.resultFiles.textContent = "1";
  // Bars reflect the as-yet-uncompressed state.
  els.barOriginal.style.width = "100%";
  els.barCompressed.style.width = "100%";
}

function showResultEmpty() {
  els.resultEmpty.hidden = false;
  els.resultFilled.hidden = true;
  els.cardFiles.hidden = true;
}

// -----------------------------------------------------------------------
// Big result dashboard (after compress)
// -----------------------------------------------------------------------
function showResult(origSize, compSize, ratio, timeMs, nFiles = 1) {
  els.resultEmpty.hidden = true;
  els.resultFilled.hidden = false;

  els.resultOrig.textContent = fmtBytes(origSize);
  els.resultComp.textContent = fmtBytes(compSize);
  els.resultRatio.textContent = fmtRatio(ratio);
  els.resultTime.textContent = fmtMs(timeMs);
  els.resultFiles.textContent = String(nFiles);

  const saved = Math.max(0, origSize - compSize);
  const savedPct = origSize > 0 ? saved / origSize : 0;
  els.resultSavedVal.textContent = fmtBytes(saved);
  els.resultSavedPct.textContent = fmtPct(savedPct);

  // The visual bar: original is full width, compressed is ratio-shared.
  const widthPct = origSize > 0 ? Math.max(2, (compSize / origSize) * 100) : 0;
  // Animate to the new width on the next frame.
  requestAnimationFrame(() => {
    els.barCompressed.style.width = `${widthPct}%`;
  });
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
  els.filesHint.textContent = `${result.n_files} files · avg ${fmtRatio(result.aggregate_ratio)}`;
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
// Compress (single file)
// -----------------------------------------------------------------------
els.btnCompress.addEventListener("click", async () => {
  if (!lastFile && !lastDirResult) return;
  if (lastDirResult && lastArchive) {
    // Re-show the last folder result without recompressing.
    showResult(
      lastDirResult.total_original_size,
      lastDirResult.total_compressed_size,
      lastDirResult.aggregate_ratio,
      lastDirResult.total_time_ms,
      lastDirResult.n_files,
    );
    return;
  }
  if (!lastFile) return;

  setStatus("working", "compressing…");
  els.dropzone.classList.add("processing");
  hideResultFilled();

  const level = levelName();
  const t0 = performance.now();
  try {
    const input = Array.from(lastFile.bytes);
    const res = await invoke("compress_bytes_cmd", { input });
    const wall = performance.now() - t0;

    showResult(
      res.original_size,
      res.compressed_size,
      res.ratio,
      res.compress_time_ms,
      1,
    );
    log(
      "ok",
      `compressed in <strong>${fmtMs(res.compress_time_ms)}</strong> · ` +
        `ratio <strong>${fmtRatio(res.ratio)}</strong> · ` +
        `<strong>${fmtBytes(res.original_size)}</strong> → <strong>${fmtBytes(res.compressed_size)}</strong>`,
      true,
    );
    log(
      "info",
      `level=${level} · wall ${fmtMs(wall)} · ` +
        `IPC ${fmtMs(Math.max(0, wall - res.compress_time_ms))}`,
    );
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
});

function hideResultFilled() {
  els.resultFilled.hidden = true;
  els.resultEmpty.hidden = false;
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
    log("info", "check capabilities/default.json for dialog:default");
    setStatus("err", "picker failed");
    return;
  }
  if (!picked) {
    log("info", "folder picker cancelled");
    setStatus("ready", "ready");
    return;
  }
  log("rx", `picked: <strong>${picked}</strong>`);

  setStatus("working", "compressing folder…");
  els.dropzone.classList.add("processing");
  hideResultFilled();
  els.cardFiles.hidden = true;

  const level = levelName();
  const t0 = performance.now();
  try {
    const result = await invoke("compress_directory_cmd", {
      inputDir: picked,
      level,
    });
    const [dirResult, archive] = result;
    const wall = performance.now() - t0;

    showResult(
      dirResult.total_original_size,
      dirResult.total_compressed_size,
      dirResult.aggregate_ratio,
      dirResult.total_time_ms,
      dirResult.n_files,
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
    log(
      "info",
      `level=${level} · wall ${fmtMs(wall)} · ` +
        `IPC ${fmtMs(Math.max(0, wall - dirResult.total_time_ms))}`,
    );
    lastDirResult = dirResult;
    lastArchive = new Uint8Array(archive);
    lastDirPath = picked;
    renderFileList(dirResult);
    els.dropzoneMeta.textContent =
      `${picked.split("/").pop()} · ${dirResult.n_files} files · ${fmtBytes(dirResult.total_original_size)}`;
    els.actionMeta.innerHTML =
      `<strong>${picked.split("/").pop()}</strong> · ${dirResult.n_files} files · ${fmtBytes(dirResult.total_original_size)}`;
    els.actionStatus.innerHTML =
      `<strong style="color:var(--success)">done</strong> · ${fmtRatio(dirResult.aggregate_ratio)}`;
    updateCompressButton();
    setStatus("ok", "compressed");
  } catch (e) {
    console.error("[nexus] compress_directory_cmd failed:", e);
    log("err", `folder compress failed: <strong>${e}</strong>`);
    setStatus("err", "folder failed");
  } finally {
    els.dropzone.classList.remove("processing");
  }
}

// -----------------------------------------------------------------------
// Save archive (browser download, lands in Downloads folder)
// -----------------------------------------------------------------------
els.btnSaveArchive.addEventListener("click", async () => {
  let bytes, name;
  if (lastArchive && lastDirPath) {
    bytes = lastArchive;
    name = (lastDirPath.split("/").pop() || "folder") + ".nxar";
  } else if (lastOutput && lastOutput.isCompressed) {
    bytes = lastOutput.bytes;
    name = lastOutput.name;
  } else {
    log("warn", "no archive to save — compress something first");
    return;
  }
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
  setStatus("ok", "saved");
});

// -----------------------------------------------------------------------
// "Compress another" — clear the state and return to empty
// -----------------------------------------------------------------------
els.btnAnother.addEventListener("click", () => {
  lastFile = null;
  lastOutput = null;
  lastDirResult = null;
  lastArchive = null;
  lastDirPath = null;
  showResultEmpty();
  els.cardFiles.hidden = true;
  els.dropzoneMeta.textContent = "supports folders · .nxr archives · any file type";
  els.actionMeta.innerHTML = `<span class="muted">drop a folder or file to begin</span>`;
  els.actionStatus.innerHTML = `<span class="muted">idle</span>`;
  updateCompressButton();
  setStatus("ready", "ready");
  log("info", "ready for the next one");
});

// -----------------------------------------------------------------------
// Self-test
// -----------------------------------------------------------------------
els.btnSelfTest.addEventListener("click", async () => {
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
});

// -----------------------------------------------------------------------
// Engine info on boot
// -----------------------------------------------------------------------
async function loadEngineInfo() {
  try {
    const info = await invoke("engine_info_cmd");
    log(
      "info",
      `engine v${info.version} · format v${info.format_version} · ${info.dict_entries} dict entries`,
    );
    setStatus("ready", "ready");
  } catch (e) {
    log("err", `engine_info failed: ${e}`);
    setStatus("err", "engine offline");
  }
}

window.addEventListener("DOMContentLoaded", () => {
  log("info", "NexusRAR booting…");
  updateCompressButton();
  loadEngineInfo();
});
