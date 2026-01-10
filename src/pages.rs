pub(crate) fn render_app_page(is_authed: bool, show_login_error: bool) -> String {
    let css = include_str!("../static/styles.css");
    let js = include_str!("../static/app.js");

    let auth_data = if is_authed { "1" } else { "0" };
    let login_error_data = if show_login_error { "1" } else { "0" };
    let auth_action_html = if is_authed {
        r#"<form method="post" action="/logout">
          <button class="btn ghost" type="submit">Log out</button>
        </form>"#
    } else {
        r#"<button id="openAuthBtn" class="btn primary" type="button">Sign in</button>"#
    };

    let dropzone_title = if is_authed {
        "Drag & drop up to 10 PDF files"
    } else {
        "Sign in to upload PDFs"
    };
    let dropzone_sub = if is_authed {
        "…or click to choose files"
    } else {
        "Click to sign in"
    };

    format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>PDF Tools</title>
    <style>{css}</style>
  </head>
  <body data-authed="{auth_data}" data-login-error="{login_error_data}">
    <div class="bg"></div>
    <main class="shell">
      <header class="topbar">
        <div class="brand">
          <div class="logo" aria-hidden="true">PDF</div>
          <div>
            <div class="title">PDF Tools</div>
            <div class="subtitle">Merge PDFs in order, pick output quality</div>
          </div>
        </div>
        {auth_action_html}
      </header>

      <section class="grid">
        <div class="card">
          <div class="section-title">Upload PDFs</div>
          <div id="dropzone" class="dropzone" tabindex="0" role="button" aria-label="Upload PDFs">
            <div class="dz-title">{dropzone_title}</div>
            <div class="dz-sub">{dropzone_sub}</div>
          </div>
          <input id="fileInput" type="file" accept="application/pdf,.pdf" multiple hidden />

          <div class="list-head">
            <div class="muted">Order matters (drag to reorder)</div>
            <div class="muted"><span id="count">0</span>/10</div>
          </div>
          <ul id="fileList" class="file-list" aria-label="Uploaded PDFs"></ul>
          <div id="empty" class="empty">No files yet.</div>
        </div>

        <div class="card">
          <div class="section-title">Output settings</div>
          <div class="row">
            <div>
              <div class="label-row">
                <div class="label">Quality</div>
                <div class="pill"><span id="qualityValue">80</span>%</div>
              </div>
              <input id="quality" class="range" type="range" min="10" max="100" value="80" />
            </div>
          </div>

          <div class="stats">
            <div class="stat">
              <div class="stat-k">Input size</div>
              <div class="stat-v" id="inputSize">0 B</div>
            </div>
            <div class="stat">
              <div class="stat-k">Estimated output</div>
              <div class="stat-v" id="estimatedSize">0 B</div>
            </div>
          </div>

          <div class="row" style="margin-top:12px">
            <label class="toggle">
              <input id="linearize" type="checkbox" />
              <span class="switch" aria-hidden="true"></span>
              <span class="toggle-text">Linearize (fast web view)</span>
            </label>
          </div>

          <div class="actions">
            <button id="mergeBtn" class="btn primary cta" type="button" disabled>Download</button>
            <button id="clearBtn" class="btn" type="button" disabled>Clear</button>
          </div>
          <div class="hint">Nothing is stored server-side; refresh clears the workspace.</div>

          <div id="toast" class="toast" role="status" aria-live="polite"></div>
        </div>
      </section>
    </main>

    <div id="authModal" class="modal-backdrop" hidden>
      <section class="modal card" role="dialog" aria-modal="true" aria-labelledby="authTitle">
        <div class="modal-head">
          <div class="modal-title" id="authTitle">Sign in to upload</div>
          <button id="authCloseBtn" class="btn icon-btn" type="button" aria-label="Close">
            <span aria-hidden="true">×</span>
          </button>
        </div>
        <div id="authError" class="alert" role="alert" hidden></div>
        <form id="authForm" class="form" method="post" action="/login" autocomplete="off">
          <label class="label" for="authUsername">Username</label>
          <input id="authUsername" class="input" name="username" autocomplete="username" required />
          <label class="label" for="authPassword">Password</label>
          <input id="authPassword" class="input" type="password" name="password" autocomplete="current-password" required />
          <button class="btn primary" type="submit">Sign in</button>
        </form>
        <div class="hint access-row">
          <span>No login details?</span>
          <a id="requestAccessLink" class="access-link" href="mailto:ihar.yazerski@gmail.com">Request access</a>
          <span class="access-sep">or email me at</span>
          <span class="email-text">ihar.yazerski@gmail.com</span>
        </div>
      </section>
    </div>

    <script>{js}</script>
  </body>
</html>"#
    )
}
