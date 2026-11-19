# Builds the Tomo API as a small container. Used by Render (see render.yaml).
# Migrations are compiled into the binary and run on startup, so nothing else is needed.

FROM rust:1-bookworm AS build
WORKDIR /app
# Some crypto crates compile C code, so make sure the build tools are there.
RUN apt-get update && apt-get install -y --no-install-recommends cmake pkg-config && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY migrations ./migrations
RUN cargo build --release -p tomo-api

FROM debian:bookworm-slim
# CA certificates are needed to reach Neon and Discord over TLS.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/target/release/tomo-api /usr/local/bin/tomo-api
# Don't run as root.
RUN useradd --system --uid 10001 tomo
USER tomo
# Render injects PORT; the app reads it (default 8080).
EXPOSE 8080
CMD ["tomo-api"]
