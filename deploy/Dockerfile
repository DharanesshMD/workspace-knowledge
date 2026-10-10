FROM node:22-bookworm-slim AS console
WORKDIR /build/web
COPY web/package*.json ./
RUN npm ci
COPY web/ ./
RUN npm run build
FROM rust:1.98.1-bookworm AS build
WORKDIR /build
COPY . .
COPY --from=console /build/web/dist ./web/dist
RUN cargo build --locked --release -p wk-server
FROM build AS pilot-build
RUN cargo build --locked --release -p wk-server --example hosted_pilot
FROM debian:bookworm-slim AS runtime-base
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* && useradd --system --uid 10001 wk
USER 10001:10001
FROM runtime-base AS pilot
COPY --from=pilot-build /build/target/release/examples/hosted_pilot /usr/local/bin/hosted_pilot
COPY --from=build /build/target/release/wk-server /usr/local/bin/wk-server
ENV WK_PILOT_BIND=0.0.0.0:8080
ENTRYPOINT ["hosted_pilot"]
FROM runtime-base AS runtime
COPY --from=build /build/target/release/wk-server /usr/local/bin/wk-server
COPY --from=console /build/web/dist /opt/wk/web
USER 10001:10001
ENV WK_BIND=0.0.0.0:8080 WK_WEB_DIR=/opt/wk/web
EXPOSE 8080
ENTRYPOINT ["wk-server"]
CMD ["--production"]
