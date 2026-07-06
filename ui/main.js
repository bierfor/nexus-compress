// NexusRAR — UI controller.
//
// Talks to the Rust backend via Tauri IPC (`window.__TAURI__.core.invoke`).
// No bundler, no framework. Plain ES2022 modules.

const { invoke } = window.__TAURI__.core;

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

  resultStrip: $("result-strip"),
  resultOrig: $("result-orig"),
  resultComp: $("result-comp"),
  resultRatio: $("result-ratio"),
  resultTime: $("result-time"),

  console: $("console"),
  telemetryMeta: $("telemetry-meta"),
  features: $("features"),
};

// -----------------------------------------------------------------------
// State
// -----------------------------------------------------------------------
let lastFile = null; // { name, bytes: Uint8Array }
let lastOutput = null; // { name, bytes: Uint8Array, isCompressed: bool }

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
els.dropzone.addEventListener("click", () => els.fileInput.click());
els.dropzone.addEventListener("keydown", (e) => {
  if (e.key === "Enter" || e.key === " ") {
    e.preventDefault();
    els.fileInput.click();
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
  const f = e.dataTransfer.files[0];
  if (f) loadFile(f);
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
  hideResult();
  setStatus("ok", "ready");
  log("ok", "file ready · click COMPRESS to run the engine");
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
els.btnDecompress.addEventListener("click", async () => {
  if (!lastFile) return;
  setStatus("working", "decompressing…");
  els.dropzone.classList.add("processing");
  hideResult();
  log("tx", `decompress ${lastFile.name}`);

  const t0 = performance.now();
  try {
    const input = Array.from(lastFile.bytes);
    const res = await invoke("decompress_bytes_cmd", { input });

    const t1 = performance.now();
    const wall = t1 - t0;
    showResult(
      res.size,
      lastFile.bytes.length,
      lastFile.bytes.length > 0 ? lastFile.bytes.length / res.size : 0,
      res.decompress_time_ms,
    );
    log(
      "ok",
      `decompressed in <strong>${fmtMs(res.decompress_time_ms)}</strong> ` +
        `· <strong>${fmtBytes(lastFile.bytes.length)}</strong> → <strong>${fmtBytes(res.size)}</strong>`,
      true,
    );
    log(
      "info",
      `wall ${fmtMs(wall)} · ` +
        `engine ${fmtMs(res.decompress_time_ms)} · ` +
        `IPC overhead ${fmtMs(Math.max(0, wall - res.decompress_time_ms))}`,
    );
    lastOutput = {
      name: lastFile.name.replace(/\.nxr$/, ""),
      bytes: new Uint8Array(res.data),
      isCompressed: false,
    };
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

window.addEventListener("DOMContentLoaded", () => {
  log("info", "NexusRAR booting…");
  loadEngineInfo();
});
