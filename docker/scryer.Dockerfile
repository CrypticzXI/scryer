FROM alpine:3

ARG TARGETARCH

RUN apk add --no-cache ca-certificates tzdata

WORKDIR /app

COPY ${TARGETARCH}/scryer-* /opt/scryer/

RUN chmod +x /opt/scryer/scryer-* \
    && mkdir -p /config /data

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
