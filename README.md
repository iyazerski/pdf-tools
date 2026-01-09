# pdf-tools

Minimal PDF merge web app (Rust + axum) with a simple login, drag&drop upload, ordering, and a quality slider.

## Features

- Authenticated web UI (username/password from env, no DB)
- Upload up to 10 PDFs via drag&drop
- Reorder PDFs (drag or Up/Down)
- Page-level editing (expand document, reorder/remove pages, insert another document between pages)
- Merge into a single PDF in the selected order
- Quality slider controls Ghostscript downsampling/JPEG quality
- No persistence: nothing stored beyond each request; refresh clears client-side list

## Local run (Docker)

```bash
docker build -t pdf-tools:local .
docker run --rm -p 8080:8080 \
  -e APP_USERNAME=admin \
  -e APP_PASSWORD=admin \
  -e SESSION_SECRET="change-me-please" \
  pdf-tools:local
```

Open `http://localhost:8080`.

## Local run (Cargo)

Create a `.env` file (the app automatically reads it on startup) and run:

Prereqs:

- Rust
- Ghostscript (`gs`)
- qpdf (`qpdf`)

```bash
cargo run
```

## Development

### pre-commit

Install `pre-commit` (e.g. `brew install pre-commit` or `pip install pre-commit`) and enable hooks:

```bash
pre-commit install
```

Hooks run `cargo fmt`, `cargo clippy` and `cargo check --locked` on commit.

## Configuration

- `APP_USERNAME` / `APP_PASSWORD` (required)
- `SESSION_SECRET` (required; random long string)
- `BIND_ADDR` (default `0.0.0.0:8080`)

## Kubernetes + GitHub Actions

Manifests are in `k8s/`. The GitHub Actions workflow `.github/workflows/deploy.yaml`:

- builds & pushes an image to GHCR
- creates/updates `pdf-tools-secrets` in the target namespace
- applies the manifests and waits for rollout
