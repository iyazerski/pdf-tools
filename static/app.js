(() => {
  const MAX_FILES = 10;
  const MAX_FILE_BYTES = 100 * 1024 * 1024;

  const dropzone = document.getElementById("dropzone");
  const fileInput = document.getElementById("fileInput");
  const fileList = document.getElementById("fileList");
  const empty = document.getElementById("empty");
  const mergeBtn = document.getElementById("mergeBtn");
  const clearBtn = document.getElementById("clearBtn");
  const count = document.getElementById("count");
  const quality = document.getElementById("quality");
  const qualityValue = document.getElementById("qualityValue");
  const linearize = document.getElementById("linearize");
  const inputSize = document.getElementById("inputSize");
  const estimatedSize = document.getElementById("estimatedSize");
  const toast = document.getElementById("toast");
  const openAuthBtn = document.getElementById("openAuthBtn");
  const authModal = document.getElementById("authModal");
  const authCloseBtn = document.getElementById("authCloseBtn");
  const authError = document.getElementById("authError");
  const authUsername = document.getElementById("authUsername");
  const requestAccessLink = document.getElementById("requestAccessLink");

  const ACCESS_EMAIL = "ihar.yazerski@gmail.com";
  const LOGIN_ERROR_STORAGE_KEY = "pdf_tools_login_error";

  const isAuthed = document.body && document.body.dataset
    ? document.body.dataset.authed === "1"
    : false;

  function buildAccessRequestMailto() {
    const origin = window.location && window.location.origin ? window.location.origin : "";
    const subject = encodeURIComponent("Access request: PDF Tools");
    const body = encodeURIComponent(
      `Hi,\n\nPlease grant me access to PDF Tools (${origin}).\n\nThanks,\n`,
    );
    return `mailto:${ACCESS_EMAIL}?subject=${subject}&body=${body}`;
  }

  function setAuthError(message) {
    if (!authError) return;
    if (!message) {
      authError.hidden = true;
      authError.textContent = "";
      return;
    }
    authError.textContent = message;
    authError.hidden = false;
  }

  function takePendingLoginErrorFromUrl() {
    const hasLoginError = document.body && document.body.dataset
      ? document.body.dataset.loginError === "1"
      : false;
    if (!hasLoginError || isAuthed) return;

    // Don't auto-open the modal on refresh; keep the error for the next auth attempt.
    try {
      window.sessionStorage.setItem(LOGIN_ERROR_STORAGE_KEY, "1");
    } catch {
      // Ignore storage failures.
    }

    showToast("Invalid username or password.");
    if (window.history && window.history.replaceState) window.history.replaceState({}, "", "/");
  }

  function openAuthModal() {
    if (!authModal) return;
    document.body.classList.add("modal-open");
    authModal.hidden = false;
    if (requestAccessLink) requestAccessLink.href = buildAccessRequestMailto();

    let hasStoredLoginError = false;
    try {
      hasStoredLoginError = window.sessionStorage.getItem(LOGIN_ERROR_STORAGE_KEY) === "1";
    } catch {
      hasStoredLoginError = false;
    }
    if (hasStoredLoginError) {
      setAuthError("Invalid username or password.");
    } else {
      setAuthError("");
    }
    if (authUsername) authUsername.focus();
  }

  function closeAuthModal() {
    if (!authModal) return;
    authModal.hidden = true;
    document.body.classList.remove("modal-open");
    if (openAuthBtn) openAuthBtn.focus();
  }

  function requireAuthForUpload() {
    if (isAuthed) return true;
    openAuthModal();
    return false;
  }

  if (openAuthBtn) openAuthBtn.addEventListener("click", openAuthModal);
  if (authCloseBtn) authCloseBtn.addEventListener("click", closeAuthModal);
  if (requestAccessLink) requestAccessLink.href = buildAccessRequestMailto();
  if (authModal) {
    authModal.addEventListener("click", (e) => {
      if (e.target === authModal) closeAuthModal();
    });
  }
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && authModal && !authModal.hidden) closeAuthModal();
  });

  if (!isAuthed) {
    // Prevent the native file picker from being reachable in any browser.
    fileInput.disabled = true;
  }

  takePendingLoginErrorFromUrl();

  /**
   * @typedef {{ id: string, file: File, name: string, size: number, pages: number | null }} Doc
   * @typedef {{ id: string, type: "doc" | "header" | "page", docId: string, page?: number }} Node
   */

  /** @type {Map<string, Doc>} */
  const docs = new Map();
  /** @type {Node[]} */
  let nodes = [];

  let draggingEl = null;

  function uid() {
    if (globalThis.crypto && crypto.randomUUID) return crypto.randomUUID();
    return `id_${Math.random().toString(16).slice(2)}`;
  }

  function showToast(message) {
    toast.textContent = message;
    toast.classList.add("show");
    window.clearTimeout(showToast._t);
    showToast._t = window.setTimeout(() => toast.classList.remove("show"), 2600);
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
    let sum = 0;
    for (const d of docs.values()) sum += d.size;
    return sum;
  }

  function estimateOutputBytes() {
    const q = Number(quality.value);
    const t = (q - 10) / 90; // 0..1
    const factor = 0.18 + 0.88 * t; // conservative; output may be > input for already-compressed PDFs
    const est = Math.round(totalInputBytes() * factor + 24 * 1024);
    return Math.max(0, est);
  }

  function docsUsedByNodes() {
    const used = new Set();
    for (const n of nodes) used.add(n.docId);
    return used;
  }

  function allPageCountsKnown() {
    for (const docId of docsUsedByNodes()) {
      const d = docs.get(docId);
      if (!d || d.pages == null) return false;
    }
    return true;
  }

  function setUiState() {
    const hasAny = docs.size > 0;
    empty.style.display = hasAny ? "none" : "block";
    mergeBtn.disabled = !hasAny || !allPageCountsKnown();
    clearBtn.disabled = !hasAny;
    count.textContent = String(docs.size);
    inputSize.textContent = formatBytes(totalInputBytes());
    estimatedSize.textContent = formatBytes(estimateOutputBytes());
  }

  function removeDocEverywhere(docId) {
    docs.delete(docId);
    nodes = nodes.filter((n) => n.docId !== docId);
    renderList();
  }

  function maybeCleanupDoc(docId) {
    const stillUsed = nodes.some((n) => n.docId === docId);
    if (!stillUsed) docs.delete(docId);
  }

  function canCollapse(docId) {
    const d = docs.get(docId);
    if (!d || d.pages == null) return false;
    const pageNodes = nodes.filter((n) => n.type === "page" && n.docId === docId);
    if (pageNodes.length !== d.pages) return false;
    // Only allow collapse if pages are contiguous right after the header and in 1..N order.
    const headerIdx = nodes.findIndex((n) => n.type === "header" && n.docId === docId);
    if (headerIdx < 0) return false;
    for (let i = 0; i < d.pages; i += 1) {
      const n = nodes[headerIdx + 1 + i];
      if (!n || n.type !== "page" || n.docId !== docId || n.page !== i + 1) return false;
    }
    return true;
  }

  function expandDoc(docId) {
    const d = docs.get(docId);
    if (!d || d.pages == null) {
      showToast("Pages are still being calculated…");
      return;
    }
    if (d.pages <= 1) return;
    const idx = nodes.findIndex((n) => n.type === "doc" && n.docId === docId);
    if (idx < 0) return;

    const header = { id: `h_${docId}`, type: "header", docId };
    const pages = [];
    for (let p = 1; p <= d.pages; p += 1) {
      pages.push({ id: `p_${docId}_${p}`, type: "page", docId, page: p });
    }
    const copy = nodes.slice();
    copy.splice(idx, 1, header, ...pages);
    nodes = copy;
    renderList();
  }

  function collapseDoc(docId) {
    if (!canCollapse(docId)) return;
    const headerIdx = nodes.findIndex((n) => n.type === "header" && n.docId === docId);
    if (headerIdx < 0) return;
    const d = docs.get(docId);
    if (!d || d.pages == null) return;
    const copy = nodes.slice();
    copy.splice(headerIdx, d.pages + 1, { id: `d_${docId}`, type: "doc", docId });
    nodes = copy;
    renderList();
  }

  function renderList() {
    fileList.innerHTML = "";

    const icons = {
      trash:
        '<svg viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M9 3h6l1 2h5v2H3V5h5l1-2Zm1 7h2v9h-2v-9Zm4 0h2v9h-2v-9ZM6 8h12l-1 14H7L6 8Z"/></svg>',
      chevronDown:
        '<svg viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M6.7 8.7a1 1 0 0 1 1.4 0L12 12.6l3.9-3.9a1 1 0 1 1 1.4 1.4l-4.6 4.6a1 1 0 0 1-1.4 0L6.7 10.1a1 1 0 0 1 0-1.4Z"/></svg>',
      chevronUp:
        '<svg viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M6.7 15.3a1 1 0 0 0 1.4 0l3.9-3.9 3.9 3.9a1 1 0 1 0 1.4-1.4l-4.6-4.6a1 1 0 0 0-1.4 0l-4.6 4.6a1 1 0 0 0 0 1.4Z"/></svg>',
    };

    function iconBtn({ kind, label, onClick, disabled = false }) {
      const b = document.createElement("button");
      b.className = `btn icon-btn${kind === "danger" ? " danger" : ""}`;
      b.type = "button";
      b.innerHTML = kind === "chevronDown" ? icons.chevronDown
        : kind === "chevronUp" ? icons.chevronUp
          : icons.trash;
      b.setAttribute("aria-label", label);
      b.title = label;
      b.disabled = disabled;
      b.addEventListener("click", (e) => {
        e.preventDefault();
        e.stopPropagation();
        onClick();
      });
      return b;
    }

    for (const n of nodes) {
      const d = docs.get(n.docId);
      if (!d) continue;

      const li = document.createElement("li");
      li.className = `file${n.type === "page" ? " page" : ""}${n.type === "header" ? " header" : ""}`;
      li.dataset.id = n.id;
      li.draggable = n.type !== "header";

      const meta = document.createElement("div");
      meta.className = "meta";
      const name = document.createElement("div");
      name.className = "name";
      const sub = document.createElement("div");
      sub.className = "sub";

      if (n.type === "doc") {
        name.textContent = d.name;
        const pagesLabel = d.pages == null ? "pages: …" : `${d.pages} page${d.pages === 1 ? "" : "s"}`;
        sub.textContent = `${formatBytes(d.size)} · PDF · ${pagesLabel}`;
      } else if (n.type === "header") {
        name.textContent = d.name;
        sub.textContent = "Page editing (drag pages/documents to reorder)";
      } else {
        name.textContent = `Page ${n.page}`;
        sub.textContent = d.name;
      }

      meta.appendChild(name);
      meta.appendChild(sub);

      const tools = document.createElement("div");
      tools.className = "tools";

      if (n.type === "doc") {
        const canExpand = d.pages != null && d.pages > 1;
        tools.appendChild(
          iconBtn({
            kind: "chevronDown",
            label: "Show pages",
            onClick: () => expandDoc(n.docId),
            disabled: !canExpand,
          }),
        );
        tools.appendChild(
          iconBtn({
            kind: "danger",
            label: "Remove document",
            onClick: () => removeDocEverywhere(n.docId),
          }),
        );
      } else if (n.type === "header") {
        tools.appendChild(
          iconBtn({
            kind: "chevronUp",
            label: "Hide pages",
            onClick: () => collapseDoc(n.docId),
            disabled: !canCollapse(n.docId),
          }),
        );
        tools.appendChild(
          iconBtn({
            kind: "danger",
            label: "Remove document",
            onClick: () => removeDocEverywhere(n.docId),
          }),
        );
      } else {
        tools.appendChild(
          iconBtn({
            kind: "danger",
            label: "Remove page",
            onClick: () => {
              nodes = nodes.filter((x) => x.id !== n.id);
              maybeCleanupDoc(n.docId);
              renderList();
            },
          }),
        );
      }

      li.appendChild(meta);
      li.appendChild(tools);
      fileList.appendChild(li);
    }
    setUiState();
  }

  async function fetchNpages(docId) {
    const d = docs.get(docId);
    if (!d) return;
    const fd = new FormData();
    fd.append("file", d.file, d.name);
    try {
      const res = await fetch("/api/npages", {
        method: "POST",
        body: fd,
        credentials: "same-origin",
      });
      if (res.status === 401) {
        openAuthModal();
        throw new Error("Sign in to upload files.");
      }
      if (!res.ok) {
        const text = await res.text().catch(() => "");
        throw new Error(text || `Failed to read pages (${res.status})`);
      }
      const data = await res.json();
      const pages = Number(data.pages);
      if (!Number.isFinite(pages) || pages <= 0) throw new Error("Invalid pages response");
      const cur = docs.get(docId);
      if (cur) {
        cur.pages = pages;
        docs.set(docId, cur);
      }
      renderList();
    } catch (err) {
      showToast(err && err.message ? err.message : "Failed to read pages.");
      const cur = docs.get(docId);
      if (cur && cur.pages == null) {
        cur.pages = 1;
        docs.set(docId, cur);
        renderList();
      }
    }
  }

  function addFiles(fileListObj) {
    const files = Array.from(fileListObj);
    const accepted = files.filter((f) => {
      const nameOk = f.name.toLowerCase().endsWith(".pdf");
      const typeOk = !f.type || f.type === "application/pdf";
      return nameOk && typeOk;
    });
    if (accepted.length !== files.length) {
      showToast("Some files were skipped (only PDFs are allowed).");
    }
    if (accepted.length === 0) return;

    const space = MAX_FILES - docs.size;
    if (space <= 0) {
      showToast(`Max ${MAX_FILES} files.`);
      return;
    }

    const slice = accepted.slice(0, space);
    for (const f of slice) {
      if (f.size > MAX_FILE_BYTES) {
        showToast(`Skipped ${f.name} (max 100 MB per file).`);
        continue;
      }
      const docId = uid();
      docs.set(docId, { id: docId, file: f, name: f.name, size: f.size, pages: null });
      nodes.push({ id: `d_${docId}`, type: "doc", docId });
      fetchNpages(docId);
    }
    if (slice.length < accepted.length) {
      showToast(`Only the first ${slice.length} files were added (max ${MAX_FILES}).`);
    }
    renderList();
  }

  fileInput.addEventListener("change", () => {
    if (fileInput.files) addFiles(fileInput.files);
    fileInput.value = "";
  });

  dropzone.addEventListener("dragover", (e) => {
    e.preventDefault();
    if (isAuthed) dropzone.classList.add("dragover");
  });
  dropzone.addEventListener("dragleave", () => dropzone.classList.remove("dragover"));
  dropzone.addEventListener("drop", (e) => {
    e.preventDefault();
    dropzone.classList.remove("dragover");
    if (!isAuthed) {
      openAuthModal();
      return;
    }
    if (e.dataTransfer && e.dataTransfer.files) addFiles(e.dataTransfer.files);
  });

  quality.addEventListener("input", () => {
    qualityValue.textContent = quality.value;
    setUiState();
  });

  if (linearize) {
    linearize.addEventListener("change", () => setUiState());
  }

  clearBtn.addEventListener("click", () => {
    nodes = [];
    docs.clear();
    renderList();
    showToast("Cleared.");
  });

  // Drag-to-reorder within the output list (docs + pages).
  fileList.addEventListener("dragstart", (e) => {
    const li = e.target.closest(".file");
    if (!li || li.draggable === false) return;
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
    if (over.classList.contains("header")) return;
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
    const byId = new Map(nodes.map((n) => [n.id, n]));
    const next = [];
    for (const id of ids) {
      const n = byId.get(id);
      if (n) next.push(n);
    }
    nodes = next;
    setUiState();
  }

  function buildLayout() {
    /** @type {{doc: string, page: number}[]} */
    const layout = [];
    for (const n of nodes) {
      const d = docs.get(n.docId);
      if (!d || d.pages == null) continue;
      if (n.type === "doc") {
        for (let p = 1; p <= d.pages; p += 1) layout.push({ doc: n.docId, page: p });
      } else if (n.type === "page") {
        layout.push({ doc: n.docId, page: n.page });
      }
    }
    return layout;
  }

  async function doMerge() {
    if (docs.size === 0) return;
    if (!isAuthed) {
      openAuthModal();
      return;
    }
    if (!allPageCountsKnown()) {
      showToast("Pages are still being calculated…");
      return;
    }
    const layout = buildLayout();
    if (layout.length === 0) {
      showToast("Nothing to merge.");
      return;
    }

    mergeBtn.disabled = true;
    clearBtn.disabled = true;
    const prev = mergeBtn.textContent;
    mergeBtn.textContent = "Downloading…";
    try {
      const usedDocs = new Set(layout.map((x) => x.doc));
      const fd = new FormData();
      fd.append("quality", String(quality.value));
      fd.append("linearize", linearize && linearize.checked ? "1" : "0");
      fd.append("layout", JSON.stringify(layout));
      for (const docId of usedDocs) {
        const d = docs.get(docId);
        if (!d) continue;
        fd.append(`file_${docId}`, d.file, d.name);
      }

      const res = await fetch("/api/merge", {
        method: "POST",
        body: fd,
        credentials: "same-origin",
      });
      if (res.status === 401) {
        openAuthModal();
        throw new Error("Sign in to download.");
      }
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

  // Gate upload interactions last so earlier listeners remain simple.
  dropzone.addEventListener("click", (e) => {
    if (!requireAuthForUpload()) {
      e.preventDefault();
      e.stopPropagation();
      return;
    }
    fileInput.click();
  });
  dropzone.addEventListener("keydown", (e) => {
    if (e.key !== "Enter" && e.key !== " ") return;
    if (!requireAuthForUpload()) {
      e.preventDefault();
      e.stopPropagation();
      return;
    }
    fileInput.click();
  });
  fileInput.addEventListener("click", (e) => {
    if (!requireAuthForUpload()) {
      e.preventDefault();
      e.stopPropagation();
    }
  });

})();
