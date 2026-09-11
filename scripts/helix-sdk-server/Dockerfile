# The HelixDB server the `helix-sdk` backend talks to. The `helix-db` 3.0.0
# Rust SDK posts a nested query AST to `/v2/query`; the registry image the
# `helix` service runs (enterprise-dev, HTTP/v1) has no such route and answers
# every SDK request with 400 "missing field `queries`", so the SDK lane needs
# the standalone server that grew up with that SDK: HelixDB revision
# 0ef3cee0faf28bb81072fb149b982dcdb166d60a (server package 0.1.0), which the
# sibling LSQB harness qualified from source (Grust
# benchmarks/lsqb/Dockerfile.helix-sdk-server, HELIX-SDK-DOCKER.md). This is
# that recipe on amd64: the Rust base is the same multi-arch index digest; the
# distroless base is the multi-arch index (the LSQB pin is the arm64 manifest).
# Build with scripts/build-helix-sdk-server.sh; the harness records the image
# ID it ran against, not the tag.
# syntax=docker/dockerfile:1.7
FROM rust:1.97.1-bookworm@sha256:0e2bcaef56d041a486784e54104a81aebe0da44bd03019bd70bc0401e42e4a97 AS builder
WORKDIR /workspace
RUN apt-get update && apt-get install -y --no-install-recommends binutils build-essential ca-certificates clang cmake git libssl-dev pkg-config && rm -rf /var/lib/apt/lists/*
COPY . .
ARG JOBS=2
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,id=ag-helix-sdk-server,target=/workspace/target \
    cargo build --locked --release --package server --bin server -j${JOBS} \
    && cp target/release/server /tmp/helix-server
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f
COPY --from=builder /tmp/helix-server /bin/helix-server
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt HELIX_HTTP_ADDR=0.0.0.0:8080 HELIX_GRPC_ADDR=127.0.0.1:8081 DB_PATH=db/ RUST_LOG=server=warn,db=warn,slatedb=warn
LABEL org.opencontainers.image.revision="0ef3cee0faf28bb81072fb149b982dcdb166d60a" org.opencontainers.image.source="https://github.com/HelixDB/helix-db" io.adversarial.purpose="helix-sdk-server"
WORKDIR /home/nonroot
USER 65532:65532
EXPOSE 8080
STOPSIGNAL SIGTERM
ENTRYPOINT ["/bin/helix-server"]
