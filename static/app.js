(() => {
  const MAX_FILES = 10;
  const MAX_FILE_BYTES = 30 * 1024 * 1024;
  // Matches the server, which copies pages as-is at this quality, without recompression.
  const LOSSLESS_QUALITY = 100;

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
  const openAuthBtn = document.getElementById("openAuthBtn");
  const authModal = document.getElementById("authModal");
  const authCloseBtn = document.getElementById("authCloseBtn");
  const authError = document.getElementById("authError");
  const authUsername = document.getElementById("authUsername");
  const requestAccessLink = document.getElementById("requestAccessLink");
  const previewModal = document.getElementById("previewModal");
  const previewCloseBtn = document.getElementById("previewCloseBtn");
  const previewTitle = document.getElementById("previewTitle");
  const previewMeta = document.getElementById("previewMeta");
  const previewLoading = document.getElementById("previewLoading");
  const previewImg = document.getElementById("previewImg");

  const ACCESS_EMAIL = "ihar.yazerski@gmail.com";

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

  function refreshModalOpenClass() {
    const anyOpen = (authModal && !authModal.hidden) || (previewModal && !previewModal.hidden);
    document.body.classList.toggle("modal-open", Boolean(anyOpen));
  }

  // A failed login redirects to /?login_error=1. Show the error once, and drop the query so
  // a refresh doesn't show it again.
  function showLoginErrorFromUrl() {
    const hasLoginError = document.body && document.body.dataset
      ? document.body.dataset.loginError === "1"
      : false;
    if (!hasLoginError || isAuthed) return;
    if (window.history && window.history.replaceState) window.history.replaceState({}, "", "/");
    openAuthModal("Invalid username or password.");
  }

  function openAuthModal(errorMessage = "") {
    if (!authModal) return;
    closePreviewModal();
    authModal.hidden = false;
    refreshModalOpenClass();
    if (requestAccessLink) requestAccessLink.href = buildAccessRequestMailto();
    setAuthError(errorMessage);
    if (authUsername) authUsername.focus();
  }

  function closeAuthModal() {
    if (!authModal) return;
    if (authModal.hidden) return;
    authModal.hidden = true;
    refreshModalOpenClass();
    if (openAuthBtn) openAuthBtn.focus();
  }

  let previewObjectUrl = "";
  // Aborted when the preview closes or another one opens, so stale responses are ignored.
  let previewAbort = null;

  function closePreviewModal() {
    if (previewAbort) {
      previewAbort.abort();
      previewAbort = null;
    }
    if (!previewModal) return;
    previewModal.hidden = true;
    refreshModalOpenClass();
    if (previewObjectUrl) {
      URL.revokeObjectURL(previewObjectUrl);
      previewObjectUrl = "";
    }
    if (previewImg) {
      previewImg.src = "";
      previewImg.hidden = true;
    }
    if (previewLoading) previewLoading.hidden = false;
  }

  async function openPreview(docId, page) {
    if (!isAuthed) {
      openAuthModal();
      return;
    }

    const d = docs.get(docId);
    if (!d || d.status !== "ready") {
      showToast("Preview is not available yet.");
      return;
    }
    const uploadId = d.uploadId;

    if (!previewModal || !previewTitle || !previewMeta || !previewImg || !previewLoading) return;
    closeAuthModal();

    previewTitle.textContent = `Page ${page}`;
    previewMeta.textContent = d.name;
    previewImg.hidden = true;
    previewLoading.hidden = false;

    previewModal.hidden = false;
    refreshModalOpenClass();

    if (previewAbort) previewAbort.abort();
    const abort = new AbortController();
    previewAbort = abort;
    try {
      const url = `/api/page/${encodeURIComponent(uploadId)}/${page}?kind=full`;
      const res = await fetch(url, { credentials: "same-origin", signal: abort.signal });
      if (abort.signal.aborted) return;
      if (res.status === 401) {
        closePreviewModal();
        openAuthModal();
        return;
      }
      if (res.status === 404 && await reuploadExpired(d, uploadId)) {
        if (!abort.signal.aborted) openPreview(docId, page);
        return;
      }
      if (!res.ok) {
        const text = await res.text().catch(() => "");
        throw new Error(text || `Preview failed (${res.status})`);
      }
      const blob = await res.blob();
      if (abort.signal.aborted) return;
      const objectUrl = URL.createObjectURL(blob);
      if (previewObjectUrl) URL.revokeObjectURL(previewObjectUrl);
      previewObjectUrl = objectUrl;
      previewImg.src = objectUrl;
      previewImg.hidden = false;
      previewLoading.hidden = true;
    } catch (err) {
      if (abort.signal.aborted) return;
      showToast(err && err.message ? err.message : "Preview failed.");
      closePreviewModal();
    }
  }

  function requireAuthForUpload() {
    if (isAuthed) return true;
    openAuthModal();
    return false;
  }

  if (openAuthBtn) openAuthBtn.addEventListener("click", () => openAuthModal());
  if (authCloseBtn) authCloseBtn.addEventListener("click", closeAuthModal);
  if (requestAccessLink) requestAccessLink.href = buildAccessRequestMailto();
  if (authModal) {
    authModal.addEventListener("click", (e) => {
      if (e.target === authModal) closeAuthModal();
    });
  }
  if (previewModal) {
    previewModal.addEventListener("click", (e) => {
      if (e.target === previewModal) closePreviewModal();
    });
  }
  if (previewCloseBtn) previewCloseBtn.addEventListener("click", closePreviewModal);
  document.addEventListener("keydown", (e) => {
    if (e.key !== "Escape") return;
    if (previewModal && !previewModal.hidden) closePreviewModal();
    else if (authModal && !authModal.hidden) closeAuthModal();
  });

  if (!isAuthed) {
    // Prevent the native file picker from being reachable in any browser.
    fileInput.disabled = true;
  }

  showLoginErrorFromUrl();

  /**
   * A doc is "uploading", then "ready" or "failed". A ready doc goes back to "uploading"
   * when the server drops its upload, and a failed one when the user retries. pages is
   * known from the first successful upload on, and uploadId only while ready.
   *
   * @typedef {{
   *   id: string, file: File, name: string, size: number,
   *   status: "uploading" | "ready" | "failed", pages: number | null, uploadId: string | null,
   *   error: string, uploadedAt: number, upload: Promise<void> | null, probedUploadId: string | null,
   * }} Doc
   * @typedef {{ id: string, type: "doc" | "header" | "page", docId: string, page?: number }} Node
   */

  /** @type {Map<string, Doc>} */
  const docs = new Map();
  /** @type {Node[]} */
  let nodes = [];

  // The list item being dragged. While set, the list's DOM order is only a preview: a drop
  // commits it to nodes, and dragend renders from nodes again.
  let draggingEl = null;
  let merging = false;

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

  // Approximate the bytes that end up in the output: a share of each doc by pages kept.
  function usedInputBytes() {
    const pagesUsed = new Map();
    for (const n of nodes) {
      const d = docs.get(n.docId);
      if (!d) continue;
      const add = n.type === "doc" ? (d.pages || 1) : n.type === "page" ? 1 : 0;
      pagesUsed.set(n.docId, (pagesUsed.get(n.docId) || 0) + add);
    }
    let sum = 0;
    for (const [docId, used] of pagesUsed) {
      const d = docs.get(docId);
      sum += d.pages ? d.size * Math.min(1, used / d.pages) : d.size;
    }
    return sum;
  }

  function estimateOutputBytes() {
    const used = usedInputBytes();
    if (used === 0) return 0;
    const q = Number(quality.value);
    if (q >= LOSSLESS_QUALITY) return Math.round(used);
    const t = (q - 10) / 90; // 0..1
    const factor = 0.18 + 0.88 * t; // conservative; output may be > input for already-compressed PDFs
    return Math.round(used * factor + 24 * 1024);
  }

  function renderQualityLabel() {
    const q = Number(quality.value);
    qualityValue.textContent = q >= LOSSLESS_QUALITY ? "Original" : `${q}%`;
  }

  function allDocsReady() {
    for (const d of docs.values()) {
      if (d.status !== "ready") return false;
    }
    return true;
  }

  function setUiState() {
    const hasAny = docs.size > 0;
    empty.style.display = hasAny ? "none" : "block";
    mergeBtn.disabled = merging || !hasAny || !allDocsReady();
    mergeBtn.textContent = merging ? "Downloading…" : "Download";
    clearBtn.disabled = merging || !hasAny;
    // The merge request holds the uploads it names, so they must not be removed meanwhile.
    fileList.inert = merging;
    count.textContent = String(docs.size);
    inputSize.textContent = formatBytes(totalInputBytes());
    estimatedSize.textContent = formatBytes(estimateOutputBytes());
  }

  async function deleteUpload(uploadId) {
    if (!uploadId) return;
    try {
      await fetch(`/api/upload/${encodeURIComponent(uploadId)}`, {
        method: "DELETE",
        credentials: "same-origin",
      });
    } catch {
      // Best-effort cleanup only.
    }
  }

  /**
   * Uploads d's file and marks it ready or failed. Concurrent calls share one upload.
   * @param {Doc} d
   */
  function uploadDoc(d) {
    if (!d.upload) {
      d.status = "uploading";
      d.uploadId = null;
      d.error = "";
      d.upload = sendUpload(d).finally(() => {
        d.upload = null;
      });
      renderList();
    }
    return d.upload;
  }

  /** @param {Doc} d */
  async function sendUpload(d) {
    const fd = new FormData();
    fd.append("file", d.file, d.name);
    let uploadId = "";
    let pages = 0;
    let error = "";
    let needsAuth = false;
    try {
      const res = await fetch("/api/npages", {
        method: "POST",
        body: fd,
        credentials: "same-origin",
      });
      if (res.status === 401) {
        needsAuth = true;
        error = "Sign in to upload files.";
      } else if (!res.ok) {
        error = (await res.text().catch(() => "")).trim() || `Upload failed (${res.status})`;
      } else {
        const data = await res.json();
        pages = Number(data.pages);
        uploadId = typeof data.upload_id === "string" ? data.upload_id : "";
        if (!Number.isInteger(pages) || pages <= 0 || !uploadId) error = "Invalid upload response";
      }
    } catch (err) {
      error = err && err.message ? err.message : "Upload failed.";
    }

    if (docs.get(d.id) !== d) {
      // The doc was removed while uploading; free the server copy right away.
      if (uploadId) void deleteUpload(uploadId);
      return;
    }
    if (error) {
      d.status = "failed";
      d.error = error;
      if (needsAuth) openAuthModal();
    } else {
      d.status = "ready";
      d.pages = pages;
      d.uploadId = uploadId;
      d.uploadedAt = Date.now();
    }
    renderList();
  }

  // Uploads expire after idling on the server, so an older one can go missing for good
  // reason. A younger one means uploads are being evicted under load, where uploading
  // again would only evict other uploads.
  const MIN_UPLOAD_AGE_TO_RETRY_MS = 30_000;

  /**
   * Uploads d again after the server reported uploadId as missing, and reports whether d
   * is ready afterwards.
   * @param {Doc} d
   * @param {string} uploadId
   */
  async function reuploadExpired(d, uploadId) {
    if (d.status === "ready" && d.uploadId === uploadId) {
      if (Date.now() - d.uploadedAt < MIN_UPLOAD_AGE_TO_RETRY_MS) {
        d.status = "failed";
        d.uploadId = null;
        d.error = "The server dropped this file, please try again shortly.";
        renderList();
        return false;
      }
      uploadDoc(d);
    }
    if (d.upload) await d.upload;
    return docs.get(d.id) === d && d.status === "ready";
  }

  /**
   * Checks why a thumbnail of uploadId failed to load, and uploads d again if the upload is
   * gone. Images don't expose their status, so this fetches the thumbnail once more.
   * @param {Doc} d
   * @param {string} uploadId
   * @param {string} url
   */
  async function checkThumbError(d, uploadId, url) {
    // Many thumbnails of one upload fail together, so check only the first.
    if (d.status !== "ready" || d.uploadId !== uploadId || d.probedUploadId === uploadId) return;
    d.probedUploadId = uploadId;
    try {
      const res = await fetch(url, { credentials: "same-origin" });
      if (res.status === 404) await reuploadExpired(d, uploadId);
    } catch {
      // The thumbnail stays broken, and a preview or merge reports the problem.
    }
  }

  function removeDocEverywhere(docId) {
    const d = docs.get(docId);
    if (d && d.uploadId) void deleteUpload(d.uploadId);
    docs.delete(docId);
    nodes = nodes.filter((n) => n.docId !== docId);
    renderList();
  }

  function removePage(nodeId, docId) {
    nodes = nodes.filter((n) => n.id !== nodeId);
    // A header alone adds no pages to the output, so the doc is gone with its last page.
    const hasPages = nodes.some((n) => n.docId === docId && n.type !== "header");
    if (hasPages) renderList();
    else removeDocEverywhere(docId);
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
    // Replacing the dragged element would break the drag, and dragend renders anyway.
    if (draggingEl) return;
    fileList.innerHTML = "";

    const icons = {
      trash:
        '<svg viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M9 3h6l1 2h5v2H3V5h5l1-2Zm1 7h2v9h-2v-9Zm4 0h2v9h-2v-9ZM6 8h12l-1 14H7L6 8Z"/></svg>',
      chevronDown:
        '<svg viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M6.7 8.7a1 1 0 0 1 1.4 0L12 12.6l3.9-3.9a1 1 0 1 1 1.4 1.4l-4.6 4.6a1 1 0 0 1-1.4 0L6.7 10.1a1 1 0 0 1 0-1.4Z"/></svg>',
      chevronUp:
        '<svg viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M6.7 15.3a1 1 0 0 0 1.4 0l3.9-3.9 3.9 3.9a1 1 0 1 0 1.4-1.4l-4.6-4.6a1 1 0 0 0-1.4 0l-4.6 4.6a1 1 0 0 0 0 1.4Z"/></svg>',
      retry:
        '<svg viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M12 5V2L7 6l5 4V7a5 5 0 1 1-5 5H5a7 7 0 1 0 7-7Z"/></svg>',
    };

    function iconBtn({ kind, label, onClick, disabled = false }) {
      const b = document.createElement("button");
      b.className = `btn icon-btn${kind === "danger" ? " danger" : ""}`;
      b.type = "button";
      b.innerHTML = kind === "danger" ? icons.trash : icons[kind];
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
      li.className = `file${n.type === "page" ? " page" : ""}${n.type === "header" ? " header" : ""}${d.status === "failed" ? " failed" : ""}`;
      li.dataset.id = n.id;
      li.draggable = n.type !== "header";

      if (n.type === "page") {
        const thumb = document.createElement("button");
        thumb.type = "button";
        thumb.className = "thumb-btn";
        const canPreview = d.status === "ready";
        thumb.disabled = !canPreview;
        thumb.setAttribute("aria-label", `Preview page ${n.page}`);
        thumb.title = canPreview ? "Preview page" : "Preview not available yet";
        thumb.addEventListener("click", (e) => {
          e.preventDefault();
          e.stopPropagation();
          if (!canPreview) {
            showToast("Preview is not available yet.");
            return;
          }
          openPreview(n.docId, n.page);
        });

        if (canPreview) {
          const img = document.createElement("img");
          img.className = "thumb-img";
          img.loading = "lazy";
          img.decoding = "async";
          img.alt = "";
          const uploadId = d.uploadId;
          img.src = `/api/page/${encodeURIComponent(uploadId)}/${n.page}?kind=thumb`;
          img.addEventListener("error", () => checkThumbError(d, uploadId, img.src));
          thumb.appendChild(img);
        } else {
          const ph = document.createElement("div");
          ph.className = "thumb-ph";
          ph.textContent = "PDF";
          thumb.appendChild(ph);
        }

        li.appendChild(thumb);
      }

      const meta = document.createElement("div");
      meta.className = "meta";
      const name = document.createElement("div");
      name.className = "name";
      const sub = document.createElement("div");
      sub.className = "sub";

      if (n.type === "doc") {
        name.textContent = d.name;
        const parts = [formatBytes(d.size), "PDF"];
        if (d.pages != null) parts.push(`${d.pages} page${d.pages === 1 ? "" : "s"}`);
        if (d.status === "uploading") parts.push("uploading…");
        if (d.status === "failed") parts.push(d.error);
        sub.textContent = parts.join(" · ");
      } else if (n.type === "header") {
        name.textContent = d.name;
        sub.textContent = d.status === "failed" ? d.error : "Page editing (drag pages/documents to reorder)";
      } else {
        name.textContent = `Page ${n.page}`;
        sub.textContent = d.name;
      }

      meta.appendChild(name);
      meta.appendChild(sub);

      const tools = document.createElement("div");
      tools.className = "tools";

      if (n.type !== "page" && d.status === "failed") {
        tools.appendChild(
          iconBtn({
            kind: "retry",
            label: "Upload again",
            onClick: () => uploadDoc(d),
          }),
        );
      }
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
            onClick: () => removePage(n.id, n.docId),
          }),
        );
      }

      li.appendChild(meta);
      li.appendChild(tools);
      fileList.appendChild(li);
    }
    setUiState();
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

    const sized = accepted.filter((f) => f.size <= MAX_FILE_BYTES);
    if (sized.length !== accepted.length) {
      showToast("Some files were skipped (max 30 MB per file).");
    }
    const slice = sized.slice(0, space);
    for (const f of slice) {
      const docId = uid();
      /** @type {Doc} */
      const d = {
        id: docId, file: f, name: f.name, size: f.size,
        status: "uploading", pages: null, uploadId: null,
        error: "", uploadedAt: 0, upload: null, probedUploadId: null,
      };
      docs.set(docId, d);
      nodes.push({ id: `d_${docId}`, type: "doc", docId });
      void uploadDoc(d);
    }
    if (slice.length < sized.length) {
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
    renderQualityLabel();
    setUiState();
  });

  clearBtn.addEventListener("click", () => {
    for (const d of docs.values()) {
      if (d.uploadId) void deleteUpload(d.uploadId);
    }
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

  // Renders from nodes, which also drops the preview order of a canceled drag.
  fileList.addEventListener("dragend", () => {
    draggingEl = null;
    renderList();
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
    if (draggingEl) commitOrderFromDom();
  });

  function commitOrderFromDom() {
    const order = new Map(Array.from(fileList.children, (li, i) => [li.dataset.id, i]));
    const position = (n) => order.get(n.id) ?? order.size;
    nodes = nodes.slice().sort((a, b) => position(a) - position(b));
  }

  function buildLayout() {
    /** @type {{upload: string, page: number}[]} */
    const layout = [];
    for (const n of nodes) {
      const d = docs.get(n.docId);
      if (n.type === "doc") {
        for (let p = 1; p <= d.pages; p += 1) layout.push({ upload: d.uploadId, page: p });
      } else if (n.type === "page") {
        layout.push({ upload: d.uploadId, page: n.page });
      }
    }
    return layout;
  }

  async function doMerge() {
    if (merging || docs.size === 0) return;
    if (!isAuthed) {
      openAuthModal();
      return;
    }
    if (!allDocsReady()) {
      showToast("Some files are not uploaded yet.");
      return;
    }

    merging = true;
    setUiState();
    try {
      const res = await requestMerge(Number(quality.value));
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = "merged.pdf";
      document.body.appendChild(a);
      a.click();
      a.remove();
      // Revoking synchronously can cancel the download in Firefox and Safari.
      window.setTimeout(() => URL.revokeObjectURL(url), 60_000);
      showToast("Downloaded.");
    } catch (err) {
      showToast(err && err.message ? err.message : "Merge failed.");
    } finally {
      merging = false;
      setUiState();
    }
  }

  // Sends the merge request, uploading files again when the server has dropped them.
  // reuploadExpired fails for uploads that go missing again soon, so the loop ends.
  async function requestMerge(q) {
    for (;;) {
      const res = await fetch("/api/merge", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ quality: q, layout: buildLayout() }),
        credentials: "same-origin",
      });
      if (res.status === 401) {
        openAuthModal();
        throw new Error("Sign in to download.");
      }
      if (res.status === 404) {
        const missing = res.headers.get("Missing-Upload");
        const d = Array.from(docs.values()).find((x) => x.uploadId === missing);
        if (d && await reuploadExpired(d, missing)) continue;
      }
      if (!res.ok) {
        const text = await res.text().catch(() => "");
        throw new Error(text || `Merge failed (${res.status})`);
      }
      return res;
    }
  }

  mergeBtn.addEventListener("click", doMerge);
  renderQualityLabel();
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
