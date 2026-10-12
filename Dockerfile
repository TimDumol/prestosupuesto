FROM node:24-bookworm-slim AS web-build
WORKDIR /build/web
COPY web/package.json ./
COPY web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM rust:slim-bookworm AS rust-build
WORKDIR /build/server
COPY server/ ./
RUN cargo build --release

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /app/data /app/web/dist \
    && chown -R 10001:10001 /app
WORKDIR /app
COPY --from=rust-build /build/server/target/release/presto-finance-server /app/presto-finance-server
COPY --from=rust-build /build/server/target/release/presto-admin /app/presto-admin
COPY --from=web-build --chown=10001:10001 /build/web/dist/ /app/web/dist/
ENV BIND_ADDR=0.0.0.0:3000 \
    DATABASE_PATH=/app/data/presto.sqlite \
    STATIC_DIR=/app/web/dist
EXPOSE 3000
USER 10001:10001
CMD ["/app/presto-finance-server"]
