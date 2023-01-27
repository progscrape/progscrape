FROM rust:1.67.0 as builder

COPY Cargo.* /build/
COPY application /build/application/
COPY resource /build/resource/
COPY scrapers /build/scrapers/
COPY web /build/web/
RUN cd /build && cargo build --release
RUN ls -l /build/target/release/progscrape-web

FROM rust:1.67.0
COPY --from=builder /build/target/release/progscrape-web /usr/local/bin/
COPY --from=builder /build/resource/ /var/progscrape/resource/
