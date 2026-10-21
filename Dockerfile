FROM lukemathwalker/cargo-chef:latest-rust-1.92-trixie AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Build dependencies in their own layer so source-only changes reuse it.
FROM chef AS build
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked

FROM debian:trixie-slim

RUN apt-get update \
  && apt-get install -y --no-install-recommends ghostscript qpdf ca-certificates \
  && rm -rf /var/lib/apt/lists/*

COPY --from=build /app/target/release/pdf-tools /usr/local/bin/pdf-tools
COPY --from=build /app/static /home/app/static
COPY --from=build /app/templates /home/app/templates

RUN useradd -u 10001 -U -m -d /home/app -s /usr/sbin/nologin app

WORKDIR /home/app

ENV RUST_LOG=info
EXPOSE 8091
USER 10001:10001
CMD ["pdf-tools"]
