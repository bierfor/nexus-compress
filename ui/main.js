// NexusRAR — UI controller.
//
// Talks to the Rust backend via Tauri IPC. In Tauri 2.x the global
// `window.__TAURI__` is only present when `app.withGlobalTauri` is
// set in tauri.conf.json — we don't enable that, so we call the
// low-level `window.__TAURI_INTERNALS__.invoke` directly. Same
// signature as the high-level `invoke` from `@tauri-apps/api/core`.
// No bundler, no framework. Plain ES2022 modules.

const invoke = window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);

// -----------------------------------------------------------------------
// DOM refs
// -----------------------------------------------------------------------
const $ = (id) => document.getElementById(id);

const els = {
  statusPill: $("status-pill"),
  statusDot: $("status-dot"),
  statusText: $("status-text"),

  dropzone: $("dropzone"),
  fileInput: $("file-input"),
  dropSecondary: $("dropzone-secondary"),
  sourceMeta: $("source-meta"),

  levelSlider: $("level-slider"),
  levelDesc: $("level-desc"),
  levelTicks: document.querySelectorAll(".tick"),

  btnCompress: $("btn-compress"),
  btnDecompress: $("btn-decompress"),
  btnSelfTest: $("btn-self-test"),
  btnPickFolder: $("btn-pick-folder"),
  btnSaveArchive: $("btn-save-archive"),
  btnExtractArchive: $("btn-extract-archive"),

  resultStrip: $("result-strip"),
  resultOrig: $("result-orig"),
  resultComp: $("result-comp"),
  resultRatio: $("result-ratio"),
  resultTime: $("result-time"),

  fileList: $("file-list"),
  fileListBody: $("file-list-body"),
  saveRow: $("save-row"),

  console: $("console"),
  telemetryMeta: $("telemetry-meta"),
  features: $("features"),
};

// -----------------------------------------------------------------------
// State
// -----------------------------------------------------------------------
let lastFile = null; // { name, bytes: Uint8Array }
let lastOutput = null; // { name, bytes: Uint8Array, isCompressed: bool }
let lastDirResult = null; // DirectoryResult from a compress_directory call
let lastArchive = null; // NXAR archive bytes from compress_directory
let lastDirPath = null; // { name, path } — source folder for lastDirResult

// -----------------------------------------------------------------------
// Status pill
// -----------------------------------------------------------------------
function setStatus(state, text) {
  els.statusPill.classList.remove("ok", "warn", "err", "working");
  if (state) els.statusPill.classList.add(state);
  els.statusText.textContent = text;
}

// -----------------------------------------------------------------------
// Console (telemetry feed)
// -----------------------------------------------------------------------
const TAG_STYLES = {
  info: "info",
  ok: "ok",
  warn: "warn",
  err: "err",
  rx: "rx",  // incoming (load)
  tx: "tx",  // outgoing (compress/decompress)
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
}

function clearConsole() {
  els.console.innerHTML = "";
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
  return `${Math.round(ms)} ms`;
}

function fmtRatio(r) {
  if (r <= 0) return "—";
  if (r < 1.01) return "1.00×";
  return `${r.toFixed(2)}×`;
}

// -----------------------------------------------------------------------
// Result strip
// -----------------------------------------------------------------------
function showResult(orig, comp, ratio, ms) {
  els.resultOrig.textContent = fmtBytes(orig);
  els.resultComp.textContent = fmtBytes(comp);
  els.resultRatio.textContent = fmtRatio(ratio);
  els.resultTime.textContent = fmtMs(ms);
  els.resultStrip.hidden = false;
}

function hideResult() {
  els.resultStrip.hidden = true;
}

// -----------------------------------------------------------------------
// Compression level slider
// -----------------------------------------------------------------------
els.levelSlider.addEventListener("input", () => {
  const v = Number(els.levelSlider.value);
  for (const t of els.levelTicks) {
    t.classList.toggle("active", Number(t.dataset.level) === v);
  }
  if (v === 0) {
    els.levelDesc.textContent = "fast · lazy LZ77 + 5-stream rANS";
  } else {
    els.levelDesc.textContent =
      "premium · experimental optimal DP (8.8× slower, ~0% gain)";
  }
});

// -----------------------------------------------------------------------
// Dropzone
// -----------------------------------------------------------------------
function openFilePicker() {
  // `display: none` file inputs refuse the `.click()` call on some
  // WebKit versions. The CSS now positions the input off-screen
  // with `opacity: 0` instead, which keeps the native OS picker
  // functional. If the click still fails (e.g. Tauri security
  // policy), surface the error in the telemetry console.
  try {
    els.fileInput.click();
  } catch (e) {
    log("err", `file picker failed: ${e}`);
    log("warn", "check the Tauri security CSP / capabilities for the dialog plugin");
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
  // Use the first file for v1. Folder support could enumerate but
  // the IPC payload would balloon.
  const f = els.fileInput.files[0];
  loadFile(f);
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
els.dropzone.addEventListener("drop", (e) => {
  // Tauri 2.x intercepts OS file drops before they reach the
  // webview's HTML5 `drop` event. The file picker (click on
  // dropzone) is the v1 path. Real OS drag-drop requires the
  // `tauri-plugin-fs` + `tauri-plugin-dialog` packages — see
  // README "Trade-offs / not-yet". The HTML5 drop handler is
  // here only to clean up the `.dragover` CSS class.
  e.preventDefault();
  els.dropzone.classList.remove("dragover");
});

async function loadFile(file) {
  log("rx", `loaded ${file.name} (${fmtBytes(file.size)})`);
  setStatus("working", "loading…");
  const buf = await file.arrayBuffer();
  lastFile = {
    name: file.name,
    bytes: new Uint8Array(buf),
  };
  lastOutput = null;
  els.dropSecondary.textContent = `${file.name} · ${fmtBytes(file.size)}`;
  els.sourceMeta.textContent = file.name;
  els.btnCompress.disabled = false;
  els.btnDecompress.disabled = false;
  updateDecompressButton();
  hideResult();
  setStatus("ok", "ready");
  log("ok", "file ready · click COMPRESS to run the engine");
}

/// Update the decompress button label based on what the click
/// would actually do. After a successful compress, the label
/// switches to "verify roundtrip" so the user knows the button
/// will decompress the just-produced compressed output.
function updateDecompressButton() {
  if (lastOutput && lastOutput.isCompressed) {
    els.btnDecompress.textContent = "verify roundtrip";
    els.btnDecompress.title =
      "decompress the last compressed output and confirm it roundtrips to the original";
  } else {
    els.btnDecompress.textContent = "decompress";
    els.btnDecompress.title = "decompress the loaded file (must be a NexusCompress stream)";
  }
}

// -----------------------------------------------------------------------
// Compress
// -----------------------------------------------------------------------
els.btnCompress.addEventListener("click", async () => {
  if (!lastFile) return;
  const level = Number(els.levelSlider.value) === 0 ? "fast" : "premium";
  setStatus("working", `compressing (${level})…`);
  els.dropzone.classList.add("processing");
  hideResult();
  log("tx", `compress ${lastFile.name} · level=${level}`);

  const t0 = performance.now();
  try {
    const input = Array.from(lastFile.bytes);
    const res = level === "fast"
      ? await invoke("compress_bytes_cmd", { input })
      : await invoke("compress_bytes_with_level_cmd", { input, level });

    const t1 = performance.now();
    const wall = t1 - t0;
    showResult(
      res.original_size,
      res.compressed_size,
      res.ratio,
      res.compress_time_ms,
    );
    log(
      "ok",
      `compressed in <strong>${fmtMs(res.compress_time_ms)}</strong> ` +
        `· ratio <strong>${fmtRatio(res.ratio)}</strong> ` +
        `· <strong>${fmtBytes(res.original_size)}</strong> → <strong>${fmtBytes(res.compressed_size)}</strong>`,
      true,
    );
    log(
      "info",
      `wall ${fmtMs(wall)} · ` +
        `engine ${fmtMs(res.compress_time_ms)} · ` +
        `IPC overhead ${fmtMs(Math.max(0, wall - res.compress_time_ms))}`,
    );
    lastOutput = {
      name: lastFile.name + ".nxr",
      bytes: new Uint8Array(res.compressed),
      isCompressed: true,
    };
    updateDecompressButton();
    els.telemetryMeta.textContent = "ok · " + fmtRatio(res.ratio);
    setStatus("ok", "compressed");
  } catch (e) {
    log("err", `compress failed: ${e}`);
    setStatus("err", "failed");
  } finally {
    els.dropzone.classList.remove("processing");
  }
});

// -----------------------------------------------------------------------
// Decompress
// -----------------------------------------------------------------------
//
// Smart button: if the last operation was a successful COMPRESS
// (so `lastOutput` holds a compressed stream), decompress THAT
// — this is the "verify roundtrip" path the user expects after
// clicking compress. If no compressed output exists, fall back
// to decompressing `lastFile` (the file loaded via picker /
// drop), which is the "I have a .nxr file and want to recover
// the original" path.

function pickDecompressTarget() {
  if (lastOutput && lastOutput.isCompressed) {
    return { source: lastOutput, reason: "roundtrip-verify" };
  }
  if (lastFile) {
    return { source: lastFile, reason: "input-file" };
  }
  return null;
}

els.btnDecompress.addEventListener("click", async () => {
  const target = pickDecompressTarget();
  if (!target) return;
  const { source, reason } = target;
  setStatus("working", "decompressing…");
  els.dropzone.classList.add("processing");
  hideResult();
  log(
    "tx",
    `decompress ${source.name}` +
      (reason === "roundtrip-verify" ? " · (roundtrip verify)" : ""),
  );

  const t0 = performance.now();
  try {
    const input = Array.from(source.bytes);
    const res = await invoke("decompress_bytes_cmd", { input });

    const t1 = performance.now();
    const wall = t1 - t0;
    showResult(
      res.size,
      source.bytes.length,
      source.bytes.length > 0 ? source.bytes.length / res.size : 0,
      res.decompress_time_ms,
    );
    log(
      "ok",
      `decompressed in <strong>${fmtMs(res.decompress_time_ms)}</strong> ` +
        `· <strong>${fmtBytes(source.bytes.length)}</strong> → <strong>${fmtBytes(res.size)}</strong>`,
      true,
    );
    log(
      "info",
      `wall ${fmtMs(wall)} · ` +
        `engine ${fmtMs(res.decompress_time_ms)} · ` +
        `IPC overhead ${fmtMs(Math.max(0, wall - res.decompress_time_ms))}`,
    );
    // Promote the recovered bytes to lastOutput so the user can
    // chain operations without losing the result.
    lastOutput = {
      name: source.name.replace(/\.nxr$/i, ""),
      bytes: new Uint8Array(res.data),
      isCompressed: false,
    };

    // Bonus: when this was a roundtrip verify, confirm the
    // recovered bytes are byte-identical to the original
    // (when we have one).
    if (reason === "roundtrip-verify" && lastFile) {
      const recovered = lastOutput.bytes;
      const original = lastFile.bytes;
      const sameLen = recovered.length === original.length;
      let sameContent = sameLen;
      if (sameLen) {
        for (let i = 0; i < original.length; i++) {
          if (recovered[i] !== original[i]) {
            sameContent = false;
            break;
          }
        }
      }
      if (sameContent) {
        log(
          "ok",
          `roundtrip OK · <strong>${fmtBytes(original.length)}</strong> recovered byte-identical from <strong>${fmtBytes(source.bytes.length)}</strong> compressed`,
          true,
        );
      } else {
        log(
          "err",
          `roundtrip MISMATCH · expected ${original.length} B, got ${recovered.length} B`,
        );
      }
    }

    els.telemetryMeta.textContent = "ok · decompressed";
    setStatus("ok", "decompressed");
  } catch (e) {
    log("err", `decompress failed: ${e}`);
    setStatus("err", "failed");
  } finally {
    els.dropzone.classList.remove("processing");
  }
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
// Engine info (called on startup to populate features footer)
// -----------------------------------------------------------------------
async function loadEngineInfo() {
  try {
    const info = await invoke("engine_info_cmd");
    log("info", `engine v${info.version} · format v${info.format_version} · ${info.dict_entries} dict entries`);
    const flags = [
      ["entropy-gate", info.has_entropy_gate],
      ["local-subdict", info.has_local_subdict],
      ["sparse-v3", info.has_sparse_v3],
    ];
    els.features.innerHTML = "<span>// engine</span>";
    for (const [name, on] of flags) {
      const el = document.createElement("span");
      el.className = "feat" + (on ? " on" : "");
      el.textContent = name + (on ? " ✓" : " ✗");
      els.features.appendChild(el);
    }
    setStatus("ok", "engine ready");
  } catch (e) {
    log("err", `engine_info failed: ${e}`);
    setStatus("err", "engine offline");
  }
}

// -----------------------------------------------------------------------
// Folder mode (NXAR archive)
// -----------------------------------------------------------------------

/// Show the file list for a directory compression result.
function renderFileList(result) {
  if (!result || !result.entries || result.entries.length === 0) {
    els.fileList.hidden = true;
    return;
  }
  els.fileList.hidden = false;
  // Sort: largest files first so the user sees the biggest wins.
  const entries = [...result.entries].sort(
    (a, b) => b.original_size - a.original_size,
  );
  els.fileListBody.innerHTML = "";
  for (const e of entries) {
    const row = document.createElement("div");
    row.className = "file-list-row";
    const ratio =
      e.compressed_size > 0 ? e.original_size / e.compressed_size : 0;
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
    rat.className = "file-list-ratio";
    rat.textContent = fmtRatio(ratio);
    row.append(name, orig, comp, rat);
    els.fileListBody.appendChild(row);
  }
}

/// Compress a folder picked via the OS dialog.
els.btnPickFolder.addEventListener("click", async () => {
  log("tx", "open folder picker…");
  setStatus("working", "picking folder…");
  let picked;
  try {
    picked = await invoke("pick_directory_cmd");
  } catch (e) {
    log("err", `folder picker failed: ${e}`);
    setStatus("err", "picker failed");
    return;
  }
  if (!picked) {
    log("info", "folder picker cancelled");
    setStatus("ok", "ready");
    return;
  }
  log("rx", `picked: ${picked}`);

  // Compress the folder.
  setStatus("working", "compressing folder…");
  els.dropzone.classList.add("processing");
  hideResult();
  els.fileList.hidden = true;
  els.saveRow.hidden = true;

  const level = Number(els.levelSlider.value) === 0 ? "fast" : "premium";
  const t0 = performance.now();
  try {
    const [result, archive] = await invoke("compress_directory_cmd", {
      inputDir: picked,
      level,
    });
    const wall = performance.now() - t0;
    showResult(
      result.total_original_size,
      result.total_compressed_size,
      result.aggregate_ratio,
      result.total_time_ms,
    );
    log(
      "ok",
      `compressed <strong>${result.n_files}</strong> files in <strong>${fmtMs(result.total_time_ms)}</strong> ` +
        `· ratio <strong>${fmtRatio(result.aggregate_ratio)}</strong> ` +
        `· <strong>${fmtBytes(result.total_original_size)}</strong> → <strong>${fmtBytes(result.total_compressed_size)}</strong>`,
      true,
    );
    log(
      "info",
      `wall ${fmtMs(wall)} · engine ${fmtMs(result.total_time_ms)} · ` +
        `IPC overhead ${fmtMs(Math.max(0, wall - result.total_time_ms))}`,
    );
    lastDirResult = result;
    lastArchive = new Uint8Array(archive);
    lastDirPath = picked;
    els.dropSecondary.textContent = `${picked.split("/").pop()} · ${result.n_files} files`;
    els.sourceMeta.textContent = `${result.n_files} files`;
    renderFileList(result);
    els.saveRow.hidden = false;
    els.telemetryMeta.textContent = "ok · " + fmtRatio(result.aggregate_ratio);
    setStatus("ok", "folder compressed");
  } catch (e) {
    log("err", `folder compress failed: ${e}`);
    setStatus("err", "folder failed");
  } finally {
    els.dropzone.classList.remove("processing");
  }
});

/// Save the last compressed archive to disk via the OS save dialog.
els.btnSaveArchive.addEventListener("click", async () => {
  if (!lastArchive || !lastDirPath) {
    log("warn", "no archive to save — compress a folder first");
    return;
  }
  // We can't call the dialog plugin from JS without going through
  // a Tauri command. For v1, use the simpler approach: download
  // the archive via a synthetic <a download> link. This works in
  // Tauri's webview and saves to the user's Downloads folder.
  const baseName =
    (lastDirPath.split("/").pop() || "folder") + ".nxar";
  const blob = new Blob([lastArchive], { type: "application/octet-stream" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = baseName;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  setTimeout(() => URL.revokeObjectURL(url), 5000);
  log("ok", `saved <strong>${baseName}</strong> · <strong>${fmtBytes(lastArchive.length)}</strong>`, true);
  log("info", "saved to your browser's Downloads folder");
  setStatus("ok", "archive saved");
});

/// Extract an archive: open the .nxr via the file picker, then
/// ask for an output directory, then call decompress_directory.
els.btnExtractArchive.addEventListener("click", async () => {
  if (!lastFile) {
    log("warn", "load an .nxr archive first (use the dropzone)");
    return;
  }
  // For v1: extract into a sibling folder named
  // `<archive>.extracted/` next to the original. A full output
  // directory picker is a follow-up (needs the dialog plugin's
  // pick_folder, which is already wired in Rust).
  const outDir = lastFile.name.replace(/\.nxr$/i, "") + ".extracted";
  log("tx", `extract archive → ${outDir}`);
  setStatus("working", "extracting…");
  try {
    const input = Array.from(lastFile.bytes);
    const result = await invoke("decompress_directory_cmd", {
      archive: input,
      outputDir: outDir,
    });
    log(
      "ok",
      `extracted <strong>${result.n_files}</strong> files to <strong>${outDir}</strong> in <strong>${fmtMs(result.total_time_ms)}</strong>`,
      true,
    );
    setStatus("ok", "extracted");
  } catch (e) {
    log("err", `extract failed: ${e}`);
    setStatus("err", "extract failed");
  }
});

window.addEventListener("DOMContentLoaded", () => {
  log("info", "NexusRAR booting…");
  updateDecompressButton();
  loadEngineInfo();
});
