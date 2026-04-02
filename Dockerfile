ARG RUST_VERSION=1.94.1
FROM --platform=amd64 rust:${RUST_VERSION} AS builder

# Test faster dev mode: docker buildx build --build-arg RUST_PROFILE=dev ...
ARG RUST_PROFILE=release

# Avoid interactive prompts during apt.
ENV DEBIAN_FRONTEND=noninteractive

RUN dpkg --add-architecture arm64
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    pkg-config \
    clang lld \
    parallel \
    gcc-aarch64-linux-gnu g++-aarch64-linux-gnu \
    crossbuild-essential-arm64 \
    libssl-dev libsqlite3-dev \
    libssl-dev:arm64 libsqlite3-dev:arm64 \
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

# Set up arm64 cross compile.
RUN rustup target add aarch64-unknown-linux-gnu
ENV CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
ENV CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++
RUN mkdir .cargo && echo "[target.aarch64-unknown-linux-gnu]\nlinker = \"aarch64-linux-gnu-gcc\"" > .cargo/config.toml

# pkg-config 0.3: PKG_CONFIG_PATH_${TARGET} (hyphenated triple) is selected per build script when cross-compiling.
ENV PKG_CONFIG_ALLOW_CROSS=1 \
    PKG_CONFIG_SYSROOT_DIR=/
ENV PKG_CONFIG_PATH_x86_64-unknown-linux-gnu=/usr/lib/x86_64-linux-gnu/pkgconfig
ENV PKG_CONFIG_PATH_aarch64-unknown-linux-gnu=/usr/lib/aarch64-linux-gnu/pkgconfig

# openssl-sys: target-prefixed OPENSSL_* (see its build.rs env()) before plain OPENSSL_* — parallel-safe.
ENV X86_64_UNKNOWN_LINUX_GNU_OPENSSL_LIB_DIR=/usr/lib/x86_64-linux-gnu \
    X86_64_UNKNOWN_LINUX_GNU_OPENSSL_INCLUDE_DIR=/usr/include
ENV AARCH64_UNKNOWN_LINUX_GNU_OPENSSL_LIB_DIR=/usr/lib/aarch64-linux-gnu \
    AARCH64_UNKNOWN_LINUX_GNU_OPENSSL_INCLUDE_DIR=/usr/include

RUN cargo fetch
RUN RUSTFLAGS="-Awarnings" parallel \
    --tagstring '{= s:x86_64-unknown-linux-gnu:amd64:; s:aarch64-unknown-linux-gnu:arm64: =}' \
    -j 2 --lb --tag --color \
    "cargo build --profile ${RUST_PROFILE} --target {} --target-dir target/{}" \
    ::: x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu

RUN mkdir -p /output/linux/{arm64,amd64}
RUN mv /build/target/x86_64-unknown-linux-gnu/x86_64-unknown-linux-gnu/${RUST_PROFILE}/progscrape /output/linux/amd64/progscrape-web
RUN mv /build/target/aarch64-unknown-linux-gnu/aarch64-unknown-linux-gnu/${RUST_PROFILE}/progscrape /output/linux/arm64/progscrape-web

FROM rust:${RUST_VERSION} AS tester
ARG TARGETPLATFORM
COPY --from=builder /output/$TARGETPLATFORM/progscrape-web /usr/local/bin/
COPY --from=builder /build/resource/ /var/progscrape/resource/
RUN /usr/local/bin/progscrape-web help

FROM rust:${RUST_VERSION}
ARG TARGETPLATFORM
COPY --from=tester /usr/local/bin/progscrape-web /usr/local/bin/
COPY --from=builder /build/resource/ /var/progscrape/resource/
EXPOSE 3000
VOLUME /var/progscrape/data
