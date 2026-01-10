FROM rust:1.92-bookworm AS build

WORKDIR /app
COPY . .
RUN cargo build --release --locked

FROM debian:bookworm-slim

RUN apt-get update \
  && apt-get install -y --no-install-recommends ghostscript qpdf ca-certificates \
  && rm -rf /var/lib/apt/lists/*

COPY --from=build /app/target/release/pdf-tools /usr/local/bin/pdf-tools

RUN useradd -u 10001 -U -m -d /home/app -s /usr/sbin/nologin app

ENV RUST_LOG=info
EXPOSE 8091
USER 10001:10001
CMD ["pdf-tools"]
