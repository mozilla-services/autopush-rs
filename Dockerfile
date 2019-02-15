FROM rust:1.32.0-stretch as builder

ADD . /app
WORKDIR /app
ENV PATH=$PATH:/root/.cargo/bin

RUN \
    cargo --version && \
    rustc --version && \
    mkdir -m 755 bin && \
    cargo install --path autopush --root /app


FROM debian:stretch-slim
# FROM debian:stretch  # for debugging docker build
MAINTAINER <src+push-dev@mozilla.com>
RUN \
    groupadd --gid 10001 app && \
    useradd --uid 10001 --gid 10001 --home /app --create-home app && \
    \
    apt-get -qq update && \
    apt-get -qq install -y libssl-dev ca-certificates && \
    rm -rf /var/lib/apt/lists

COPY --from=builder /app/bin /app/bin
COPY --from=builder /app/version.json /app

WORKDIR /app
# XXX: ensure we no longer bind to privileged ports and re-enable this later
#USER app

CMD ["/app/bin/autopush_rs"]
