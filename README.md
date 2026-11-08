# pdf-tools

A small web app for merging PDFs. You log in, drop in up to 10 PDFs, arrange their pages,
pick a quality, and download one merged PDF.

It's a single Go binary with no database. Uploaded files are kept in a temporary directory
for a while and then deleted.

## Features

- Upload up to 10 PDFs (30 MB each) by drag and drop.
- Reorder whole documents, or expand one to reorder, remove, or preview its pages.
- Insert pages of one document between pages of another.
- Shrink the output with a quality slider. At 100, pages are copied as-is.
- Log in with a single username and password, set in the environment.

## Quick start

You need:

- [Go](https://go.dev/dl/) (the version in `go.mod`)
- [Ghostscript](https://www.ghostscript.com) (`brew install ghostscript` on macOS). It's used
  for page previews and for compression below quality 100.

Create a `.env` file in the repo root. The app reads it on startup.

```bash
APP_USERNAME=admin
APP_PASSWORD=admin
SESSION_SECRET=any-long-random-string
```

Then run the app:

```bash
go run .
```

Open http://localhost:8091 and log in with the username and password from `.env`.

The HTML, JS, and CSS are compiled into the binary, so restart `go run .` after you change
them.

### With Docker instead

The image already includes Ghostscript:

```bash
docker build -t pdf-tools .
docker run --rm -p 8091:8091 \
  -e APP_USERNAME=admin -e APP_PASSWORD=admin -e SESSION_SECRET=any-long-random-string \
  pdf-tools
```

## How it works

1. **Upload.** The browser sends each PDF to `POST /api/npages`. The server saves it to its
   own temporary directory, counts its pages, and returns an upload ID and the page count.
2. **Preview.** Thumbnails and full-size pages come from `GET /api/page/{upload}/{page}`.
   Ghostscript renders them on first request, and the server reuses them after that.
3. **Merge.** `POST /api/merge` gets the page layout as a list of `(upload ID, page)` pairs
   and a quality. The server builds the PDF with [pdfcpu](https://github.com/pdfcpu/pdfcpu).
   Below quality 100, Ghostscript then re-encodes the images to make the file smaller.
4. **Cleanup.** An upload is deleted when you remove it in the UI, when it's idle for longer
   than `UPLOAD_CACHE_TTL_SECS`, when newer uploads push it out of the full cache, or when
   the server stops. The API answers `404` for a deleted upload, and the browser then
   uploads the file again from its own copy.

All `/api/` routes need a valid session cookie. The server runs only `MAX_PDF_JOBS` PDF jobs
at once. When it's busy, other requests wait for a free slot or get a `503`.

## Project layout

| Path | What's there |
|---|---|
| `main.go`, `config.go` | Startup, environment config, and graceful shutdown |
| `internal/server/` | HTTP routes, login, rate limiting, uploads API, previews, static assets |
| `internal/pdf/` | Page counting, merging, compression, and rendering (pdfcpu and Ghostscript) |
| `internal/uploads/` | Keeps uploaded files on disk and deletes them when they expire |
| `internal/session/` | Checks the login and signs session cookies |
| `internal/pdftest/` | Helpers that write tiny PDFs for tests |
| `templates/`, `static/` | The single-page UI (`index.html`, `app.js`, `styles.css`) |
| `k8s/`, `.github/workflows/`, `scripts/` | Deployment |

## Development

Run the tests:

```bash
go test ./...
```

Tests that need Ghostscript are skipped when `gs` isn't installed.

The repo uses [prek](https://prek.j178.dev) for git hooks (`brew install prek`). Enable
them once:

```bash
prek install
```

On every commit, the hooks run `gofmt`, `go fix`, `go mod tidy`, `govulncheck`,
`golangci-lint`, and `go test -race`. To run them by hand, use `prek run --all-files`.
CI runs the same hooks.

## Configuration

All settings come from environment variables (or `.env`).

| Variable | Default | Meaning |
|---|---|---|
| `APP_USERNAME`, `APP_PASSWORD` | required | The login |
| `SESSION_SECRET` | required | Key that signs session cookies. Use a long random string. |
| `BIND_ADDR` | `0.0.0.0:8091` | Address to listen on |
| `UPLOAD_CACHE_TTL_SECS` | `1200` | Uploads idle for this long are deleted |
| `EXTERNAL_PROCESS_TIMEOUT_SECS` | `120` | Time limit for one PDF job, and for waiting for a free slot |
| `MAX_PDF_JOBS` | `2` | PDF jobs that run at once. Each can use a full CPU and hundreds of MB. |
| `COOKIE_SECURE` | `auto` | `true`, `false`, or `auto` (secure when a trusted proxy reports HTTPS) |
| `TRUST_PROXY_HEADERS` | `false` | Trust `X-Forwarded-*` headers. Set it to `true` behind a reverse proxy. |
| `LOG_LEVEL` | `info` | `debug`, `info`, `warn`, or `error` |

## Deployment

Each push to `main` runs CI. If CI passes, the CD workflow builds an image, pushes it to
GHCR, and deploys it to a Kubernetes cluster with Traefik and cert-manager. If the rollout
or a smoke check of the public site fails, it rolls back. See `.github/workflows/cd.yaml`
and `k8s/` for the details.
