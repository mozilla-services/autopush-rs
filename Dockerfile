# NOTE: Ensure builder's Rust version matches CI's in .circleci/config.yml
# RUST_VER
FROM rust:1.83-bookworm AS builder
ARG CRATE
ARG BUILD_ARGS

ADD . /app
WORKDIR /app
ENV PATH=$PATH:/root/.cargo/bin

# cmake is required for grpcio & google-cloud-rust
RUN \
    apt-get -qq update && \
    apt-get -qq install --no-install-recommends -y cmake

RUN \
    cargo --version && \
    rustc --version && \
    mkdir -m 755 bin && \
    cargo install --path $CRATE $BUILD_ARGS --locked --root /app


FROM debian:bookworm-slim
ARG BINARY
# FROM debian:bookworm  # for debugging docker build
RUN \
    groupadd --gid 10001 app && \
    useradd --uid 10001 --gid 10001 --home /app --create-home app && \
    \
    apt-get -qq update && \
    apt-get -qq install -y libssl-dev ca-certificates && \
    rm -rf /var/lib/apt/lists

COPY --from=builder /app/bin /app/bin
COPY --from=builder /app/version.json /app
COPY --from=builder /app/entrypoint.sh /app

WORKDIR /app
# XXX: ensure we no longer bind to privileged ports and re-enable this later
#USER app

# ARG variables aren't available at runtime
ENV BINARY=/app/bin/$BINARY
ENTRYPOINT ["/app/entrypoint.sh"]
