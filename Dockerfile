ARG RUST_VERSION=1.94.1

# Builds natively for the target platform.
FROM rust:${RUST_VERSION} AS builder

# Test faster dev mode: docker buildx build --build-arg RUST_PROFILE=dev ...
ARG RUST_PROFILE=release

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates pkg-config libssl-dev libsqlite3-dev \
    && rm -rf /var/lib/apt/lists/*

# .git directory is required to serve git version
COPY .git /build/.git/
COPY Cargo.* /build/
COPY application /build/application/
COPY resource /build/resource/
COPY scrapers /build/scrapers/
COPY web /build/web/
WORKDIR /build

RUN echo "[profile.release]\nlto = true\ncodegen-units = 1" >> Cargo.toml

# `--cfg tokio_unstable` enables tokio's `taskdump` feature. The cache mounts
# persist the cargo registry and target dir across builds (per-arch gha cache),
# so the binary must be copied out of the mounted target dir within this step.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target,sharing=locked \
    RUSTFLAGS="-Awarnings --cfg tokio_unstable" cargo build --profile ${RUST_PROFILE} \
    && cp "target/$([ "$RUST_PROFILE" = dev ] && echo debug || echo "$RUST_PROFILE")/progscrape" \
       /usr/local/bin/progscrape-web

FROM rust:${RUST_VERSION} AS tester
COPY --from=builder /usr/local/bin/progscrape-web /usr/local/bin/
COPY --from=builder /build/resource/ /var/progscrape/resource/
RUN /usr/local/bin/progscrape-web help

FROM rust:${RUST_VERSION}
# Debugger for the heartbeat watchdog's native backtraces on a wedged process
# (needs CAP_SYS_PTRACE on the container too). gdb (~30MB) enables `rust-gdb`;
# swap for `lldb` (~300MB) if you want `rust-lldb`'s richer output.
RUN apt-get update && apt-get install -y --no-install-recommends gdb \
    && rm -rf /var/lib/apt/lists/*
COPY --from=tester /usr/local/bin/progscrape-web /usr/local/bin/
COPY --from=builder /build/resource/ /var/progscrape/resource/
ENV RUST_BACKTRACE=1
EXPOSE 3000
VOLUME /var/progscrape/data
