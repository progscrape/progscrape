FROM --platform=amd64 rust:1.78.0 as builder

RUN dpkg --add-architecture arm64
RUN apt-get update --allow-insecure-repositories
RUN apt install -y parallel gcc-aarch64-linux-gnu g++-aarch64-linux-gnu libssl-dev:arm64 libsqlite3-dev:arm64

# .git directory is required to serve git version
COPY .git /build/.git/
COPY Cargo.* /build/
COPY application /build/application/
COPY resource /build/resource/
COPY scrapers /build/scrapers/
COPY web /build/web/
WORKDIR /build

RUN echo "[profile.release]\nlto = true\ncodegen-units = 1" >> Cargo.toml

# Set up arm64 (hard mode)
RUN rustup target add aarch64-unknown-linux-gnu
ENV OPENSSL_INCLUDE_DIR=/usr/include/openssl/
ENV OPENSSL_LIB_DIR=/usr/lib/aarch64-linux-gnu/
RUN mkdir .cargo && echo "[target.aarch64-unknown-linux-gnu]\nlinker = \"aarch64-linux-gnu-gcc\"" > .cargo/config.toml
RUN cat .cargo/config.toml

# Build amd64 and arm64 in parallel
RUN parallel -j 2 --citation --lb --tag 'cargo build --release --target' ::: 'x86_64-unknown-linux-gnu ' 'aarch64-unknown-linux-gnu'

RUN mkdir -p /output/linux/arm64
RUN mkdir -p /output/linux/amd64
RUN mv /build/target/release/progscrape /output/linux/amd64/progscrape-web
RUN mv /build/target/aarch64-unknown-linux-gnu/release/progscrape /output/linux/arm64/progscrape-web

FROM rust:1.78.0
ARG TARGETPLATFORM
COPY --from=builder /output/$TARGETPLATFORM/progscrape-web /usr/local/bin/
COPY --from=builder /build/resource/ /var/progscrape/resource/
EXPOSE 3000
VOLUME /var/progscrape/data
