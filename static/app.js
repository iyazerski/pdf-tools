(() => {
  const MAX_FILES = 10;
  const dropzone = document.getElementById("dropzone");
  const fileInput = document.getElementById("fileInput");
  const fileList = document.getElementById("fileList");
  const empty = document.getElementById("empty");
  const mergeBtn = document.getElementById("mergeBtn");
  const clearBtn = document.getElementById("clearBtn");
  const count = document.getElementById("count");
  const quality = document.getElementById("quality");
  const qualityValue = document.getElementById("qualityValue");
  const inputSize = document.getElementById("inputSize");
  const estimatedSize = document.getElementById("estimatedSize");
  const toast = document.getElementById("toast");

  /** @type {{id: string, file: File}[]} */
  let items = [];
  let draggingEl = null;

  function showToast(message) {
    toast.textContent = message;
    toast.classList.add("show");
    window.clearTimeout(showToast._t);
    showToast._t = window.setTimeout(() => toast.classList.remove("show"), 2400);
  }

  function formatBytes(bytes) {
    const units = ["B", "KB", "MB", "GB"];
    let i = 0;
    let v = bytes;
    while (v >= 1024 && i < units.length - 1) {
      v /= 1024;
      i += 1;
    }
    return `${v.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
  }

  function totalInputBytes() {
    return items.reduce((sum, it) => sum + it.file.size, 0);
  }

  function estimateOutputBytes() {
    const q = Number(quality.value);
    const t = (q - 10) / 90; // 0..1
    const factor = 0.18 + 0.88 * t; // conservative; output may be > input for already-compressed PDFs
    const est = Math.round(totalInputBytes() * factor + 24 * 1024);
    return Math.max(0, est);
  }

  function setUiState() {
    const hasFiles = items.length > 0;
    empty.style.display = hasFiles ? "none" : "block";
    mergeBtn.disabled = !hasFiles;
    clearBtn.disabled = !hasFiles;
    count.textContent = String(items.length);
    inputSize.textContent = formatBytes(totalInputBytes());
    estimatedSize.textContent = formatBytes(estimateOutputBytes());
  }

  function renderList() {
    fileList.innerHTML = "";
    for (const it of items) {
      const li = document.createElement("li");
      li.className = "file";
      li.draggable = true;
      li.dataset.id = it.id;

      const meta = document.createElement("div");
      meta.className = "meta";
      const name = document.createElement("div");
      name.className = "name";
      name.textContent = it.file.name;
      const sub = document.createElement("div");
      sub.className = "sub";
      sub.textContent = `${formatBytes(it.file.size)} · PDF`;
      meta.appendChild(name);
      meta.appendChild(sub);

      const tools = document.createElement("div");
      tools.className = "tools";

      const up = document.createElement("button");
      up.className = "btn";
      up.type = "button";
      up.textContent = "Up";
      up.addEventListener("click", () => move(it.id, -1));

      const down = document.createElement("button");
      down.className = "btn";
      down.type = "button";
      down.textContent = "Down";
      down.addEventListener("click", () => move(it.id, 1));

      const rm = document.createElement("button");
      rm.className = "btn";
      rm.type = "button";
      rm.textContent = "Remove";
      rm.addEventListener("click", () => remove(it.id));

      tools.appendChild(up);
      tools.appendChild(down);
      tools.appendChild(rm);

      li.appendChild(meta);
      li.appendChild(tools);
      fileList.appendChild(li);
    }
    setUiState();
  }

  function move(id, delta) {
    const idx = items.findIndex((x) => x.id === id);
    if (idx < 0) return;
    const next = idx + delta;
    if (next < 0 || next >= items.length) return;
    const copy = items.slice();
    const [spliced] = copy.splice(idx, 1);
    copy.splice(next, 0, spliced);
    items = copy;
    renderList();
  }

  function remove(id) {
    items = items.filter((x) => x.id !== id);
    renderList();
  }

  function addFiles(fileList) {
    const files = Array.from(fileList);
    const accepted = files.filter((f) => {
      const nameOk = f.name.toLowerCase().endsWith(".pdf");
      const typeOk = !f.type || f.type === "application/pdf";
      return nameOk && typeOk;
    });
    if (accepted.length !== files.length) {
      showToast("Some files were skipped (only PDFs are allowed).");
    }
    if (accepted.length === 0) return;

    const space = MAX_FILES - items.length;
    if (space <= 0) {
      showToast(`Max ${MAX_FILES} files.`);
      return;
    }
    const slice = accepted.slice(0, space);
    for (const f of slice) {
      const id = (crypto && crypto.randomUUID) ? crypto.randomUUID() : String(Math.random()).slice(2);
      items.push({ id, file: f });
    }
    if (slice.length < accepted.length) {
      showToast(`Only the first ${slice.length} files were added (max ${MAX_FILES}).`);
    }
    renderList();
  }

  dropzone.addEventListener("click", () => fileInput.click());
  dropzone.addEventListener("keydown", (e) => {
    if (e.key === "Enter" || e.key === " ") fileInput.click();
  });
  fileInput.addEventListener("change", () => {
    if (fileInput.files) addFiles(fileInput.files);
    fileInput.value = "";
  });

  dropzone.addEventListener("dragover", (e) => {
    e.preventDefault();
    dropzone.classList.add("dragover");
  });
  dropzone.addEventListener("dragleave", () => dropzone.classList.remove("dragover"));
  dropzone.addEventListener("drop", (e) => {
    e.preventDefault();
    dropzone.classList.remove("dragover");
    if (e.dataTransfer && e.dataTransfer.files) addFiles(e.dataTransfer.files);
  });

  quality.addEventListener("input", () => {
    qualityValue.textContent = quality.value;
    setUiState();
  });

  clearBtn.addEventListener("click", () => {
    items = [];
    renderList();
    showToast("Cleared.");
  });

  // Drag-to-reorder within list.
  fileList.addEventListener("dragstart", (e) => {
    const li = e.target.closest(".file");
    if (!li) return;
    draggingEl = li;
    li.classList.add("dragging");
    e.dataTransfer.effectAllowed = "move";
  });

  fileList.addEventListener("dragend", () => {
    if (draggingEl) draggingEl.classList.remove("dragging");
    draggingEl = null;
    syncOrderFromDom();
  });

  fileList.addEventListener("dragover", (e) => {
    e.preventDefault();
    if (!draggingEl) return;
    const over = e.target.closest(".file");
    if (!over || over === draggingEl) return;
    const rect = over.getBoundingClientRect();
    const before = e.clientY < rect.top + rect.height / 2;
    if (before) {
      fileList.insertBefore(draggingEl, over);
    } else {
      fileList.insertBefore(draggingEl, over.nextSibling);
    }
  });

  fileList.addEventListener("drop", (e) => {
    e.preventDefault();
    syncOrderFromDom();
  });

  function syncOrderFromDom() {
    const ids = Array.from(fileList.children).map((li) => li.dataset.id);
    const byId = new Map(items.map((it) => [it.id, it]));
    const next = [];
    for (const id of ids) {
      const it = byId.get(id);
      if (it) next.push(it);
    }
    if (next.length === items.length) items = next;
    setUiState();
  }

  async function doMerge() {
    if (items.length === 0) return;
    mergeBtn.disabled = true;
    clearBtn.disabled = true;
    const prev = mergeBtn.textContent;
    mergeBtn.textContent = "Merging…";
    try {
      const fd = new FormData();
      fd.append("quality", String(quality.value));
      for (const it of items) {
        fd.append("files", it.file, it.file.name);
      }

      const res = await fetch("/api/merge", {
        method: "POST",
        body: fd,
        credentials: "same-origin",
      });
      if (!res.ok) {
        const text = await res.text().catch(() => "");
        throw new Error(text || `Merge failed (${res.status})`);
      }
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = "merged.pdf";
      document.body.appendChild(a);
      a.click();
      a.remove();
      URL.revokeObjectURL(url);
      showToast("Downloaded.");
    } catch (err) {
      showToast(err && err.message ? err.message : "Merge failed.");
    } finally {
      mergeBtn.textContent = prev;
      setUiState();
    }
  }

  mergeBtn.addEventListener("click", doMerge);

  renderList();
})();

