FROM rust:1.92-bookworm AS build

WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update \
  && apt-get install -y --no-install-recommends ghostscript qpdf ca-certificates \
  && rm -rf /var/lib/apt/lists/*

COPY --from=build /app/target/release/pdf-tools /usr/local/bin/pdf-tools

ENV RUST_LOG=info
EXPOSE 8080
CMD ["pdf-tools"]
