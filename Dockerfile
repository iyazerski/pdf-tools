FROM golang:1.27-trixie AS build
WORKDIR /src

# Download modules in their own layer so source-only changes reuse it.
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 go build -trimpath -ldflags="-s -w" -o /out/pdf-tools .

FROM debian:trixie-slim

RUN apt-get update \
  && apt-get install -y --no-install-recommends ghostscript \
  && rm -rf /var/lib/apt/lists/*

COPY --from=build /out/pdf-tools /usr/local/bin/pdf-tools

EXPOSE 8091
USER 10001:10001
CMD ["pdf-tools"]
