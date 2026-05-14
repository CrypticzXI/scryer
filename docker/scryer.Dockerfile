FROM alpine:latest

ARG TARGETARCH
ARG SCRYER_RUNTIME_MODE=default

RUN apk add --no-cache su-exec tzdata

WORKDIR /app

COPY ${TARGETARCH}/scryer-* /opt/scryer/
COPY entrypoint.sh /entrypoint.sh
COPY runtime-select.sh /runtime-select.sh
RUN case "$SCRYER_RUNTIME_MODE:$TARGETARCH" in \
      modern:amd64 | modern:arm64) rm -f /opt/scryer/scryer-portable ;; \
    esac \
 && chmod +x /entrypoint.sh /runtime-select.sh /opt/scryer/scryer-*

EXPOSE 8080

# /config holds app state: database, WASM cache, logs.
# /data is conventionally where users mount their media library.
RUN mkdir -p /config /data
VOLUME /config

ENV PUID=1000
ENV PGID=1000
ENV TZ=Etc/UTC
ENV UMASK=022
ENV SCRYER_RUNTIME_MODE=${SCRYER_RUNTIME_MODE}
ENV SCRYER_BIND=0.0.0.0:8080
ENV SCRYER_DB_PATH=/config/scryer.db
ENV EXTISM_CACHE_CONFIG=

# Graceful shutdown: let in-flight requests and background tasks finish
STOPSIGNAL SIGTERM

ENTRYPOINT ["/entrypoint.sh"]
