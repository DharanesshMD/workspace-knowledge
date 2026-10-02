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
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* && useradd --system --uid 10001 wk
COPY --from=build /build/target/release/wk-server /usr/local/bin/wk-server
COPY --from=console /build/web/dist /opt/wk/web
USER 10001:10001
ENV WK_BIND=0.0.0.0:8080 WK_WEB_DIR=/opt/wk/web
EXPOSE 8080
ENTRYPOINT ["wk-server"]
CMD ["--production"]
