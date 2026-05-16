FROM debian:bookworm-slim AS runtime-assets

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/*

FROM gcr.io/distroless/static-debian12@sha256:20bc6c0bc4d625a22a8fde3e55f6515709b32055ef8fb9cfbddaa06d1760f838

ARG TARGETARCH

WORKDIR /app

# Keep CA trust and zoneinfo explicit so future base-image swaps do not silently
# break outbound HTTPS or TZ support.
COPY --from=runtime-assets /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=runtime-assets /usr/share/zoneinfo /usr/share/zoneinfo
COPY ${TARGETARCH}/scryer-* /opt/scryer/

USER 0:0

EXPOSE 8080
VOLUME /config

ENV PUID=1000
ENV PGID=1000
ENV TZ=Etc/UTC
ENV UMASK=022
ENV SCRYER_BIND=0.0.0.0:8080
ENV SCRYER_DB_PATH=/config/scryer.db
ENV EXTISM_CACHE_CONFIG=
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt

# Graceful shutdown: let in-flight requests and background tasks finish
STOPSIGNAL SIGTERM

ENTRYPOINT ["/opt/scryer/scryer-launcher"]
