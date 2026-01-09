# pdf-tools

Minimal PDF merge web app (Rust + axum) with a simple login, drag&drop upload, ordering, and a quality slider.

## Features

- Authenticated web UI (username/password from env, no DB)
- Upload up to 10 PDFs via drag&drop
- Reorder PDFs (drag or Up/Down)
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

```bash
cargo run
```

## Configuration

- `APP_USERNAME` / `APP_PASSWORD` (required)
- `SESSION_SECRET` (required; random long string)
- `BIND_ADDR` (default `0.0.0.0:8080`)

## Kubernetes + GitHub Actions

Manifests are in `k8s/`. The GitHub Actions workflow `.github/workflows/deploy.yaml`:

- builds & pushes an image to GHCR
- creates/updates `pdf-tools-secrets` in the target namespace
- applies the manifests and waits for rollout

### Hetzner (single IP, port 8091)

Current manifests expose the app via `hostPort: 8091` on the node, so you can open:

`http://<SERVER_IP>:8091`

Required GitHub Actions secrets:

- `KUBE_CONFIG_B64` — base64-encoded kubeconfig (or `KUBE_CONFIG` as plain kubeconfig text)
- `KUBE_NAMESPACE` — optional; defaults to `default`
- `APP_USERNAME`, `APP_PASSWORD`, `SESSION_SECRET`
- `PACKAGES_PAT` — GitHub PAT with `read:packages` to create `ghcr-pull-secret` for pulling images from GHCR
- `GHCR_USERNAME` — optional; defaults to repo owner

Note: if your GHCR image is private, your cluster also needs an `imagePullSecret` configured.
