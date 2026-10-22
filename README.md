# pdf-tools

Minimal PDF merge web app (Rust + axum) with a simple login, drag&drop upload, ordering, and a quality slider.

## Features

- Authenticated web UI (username/password from env, no DB)
- Upload up to 10 PDFs via drag&drop
- Max 30 MB per PDF
- Reorder PDFs (drag to reorder)
- Page-level editing (expand document, reorder/remove pages, insert another document between pages)
- Merge into a single PDF in the selected order
- Quality slider controls Ghostscript downsampling/JPEG quality (100 = original, no recompression)
- Optional linearization for fast web view
- No persistence: nothing stored beyond each request; refresh clears client-side list

## Local run (Docker)

```bash
docker build -t pdf-tools:local .
docker run --rm -p 8091:8091 \
  -e APP_USERNAME=admin \
  -e APP_PASSWORD=admin \
  -e SESSION_SECRET="change-me-please" \
  pdf-tools:local
```

Open `http://localhost:8091`.

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

### prek

Install [prek](https://prek.j178.dev) (e.g. `brew install prek`) and enable hooks:

```bash
prek install
```

Hooks run `cargo fmt`, `cargo clippy` and `cargo check --locked` on commit.

## Configuration

- `APP_USERNAME` / `APP_PASSWORD` (required)
- `SESSION_SECRET` (required; random long string)
- `BIND_ADDR` (default `0.0.0.0:8091`)
- `UPLOAD_CACHE_TTL_SECS` (default `1200`; idle uploads are deleted after this)
- `EXTERNAL_PROCESS_TIMEOUT_SECS` (default `120`; per gs/qpdf run)
- `COOKIE_SECURE` (`auto` | `true` | `false`, default `auto`)
- `TRUST_PROXY_HEADERS` (default `false`; set when behind Traefik)

Static assets and the page template are compiled into the binary, so changes need a rebuild.

## Kubernetes + GitHub Actions

Manifests are in `k8s/`. Every push to `main` runs `.github/workflows/ci.yaml` (prek and
`cargo test`). When CI passes, `.github/workflows/cd.yaml` deploys that commit:

- builds & pushes `ghcr.io/<owner>/pdf-tools:app-<sha>` (and `app-latest`) to GHCR
- creates/updates `pdf-tools-secrets` in the target namespace
- applies a cert-manager `ClusterIssuer` for Let's Encrypt (required secret: `LETSENCRYPT_EMAIL`)
- applies the manifests, waits for rollout, and smoke-checks the public site
- restores the previous release when the rollout or smoke check fails

To redeploy manually, run the CD workflow and type `deploy` to confirm. The public host defaults to
`pdftools.iyazerski.dev` and can be changed with the `PUBLIC_HOST` repository variable.

### HTTPS (Ingress + Let's Encrypt)

This repo assumes the cluster already has:

- Traefik installed (default for k3s) exposed on ports 80/443 with entryPoints `web` and `websecure`
- cert-manager installed (to provision TLS certs via Let's Encrypt)

The app is exposed on `PUBLIC_HOST` via:

- `k8s/traefik.yaml` (middlewares + ServersTransport)
- `k8s/certificate.yaml` (cert-manager Certificate)
- `k8s/ingressroute.yaml` (Traefik IngressRoute)
