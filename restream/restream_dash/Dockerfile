FROM --platform=linux/amd64 rust:1-bookworm AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./

RUN mkdir -p src && \
    echo 'fn main() {}' > src/main.rs && \
    cargo build --release && \
    rm -f target/release/deps/restream_dash*

COPY src ./src

RUN touch src/main.rs && cargo build --release


FROM --platform=linux/amd64 debian:bookworm AS bento4-builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    cmake make g++ git ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN git clone --depth 1 https://github.com/axiomatic-systems/Bento4.git /tmp/bento4 && \
    cmake \
        -S /tmp/bento4 \
        -B /tmp/bento4/build \
        -DCMAKE_BUILD_TYPE=Release && \
    cmake --build /tmp/bento4/build --target mp4decrypt -- -j"$(nproc)" && \
    BINARY="$(find /tmp/bento4/build -maxdepth 3 -name 'mp4decrypt' -type f | head -1)" && \
    cp "$BINARY" /usr/local/bin/mp4decrypt && \
    chmod +x /usr/local/bin/mp4decrypt


FROM --platform=linux/amd64 debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ffmpeg \
    ca-certificates \
    libssl3 \
    wget \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder        /app/target/release/restream_dash /usr/local/bin/restream_dash
COPY --from=bento4-builder /usr/local/bin/mp4decrypt          /usr/local/bin/mp4decrypt

RUN useradd -r -s /bin/false -m -d /app restream && \
    mkdir -p /app/media && \
    chown -R restream:restream /app

USER restream
WORKDIR /app

VOLUME ["/app/media"]

EXPOSE 3001

ENV RUST_LOG=restream_dash=info,tower_http=warn

ENV JWT_TOKEN=eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJUb29sYm94IERpZ2l0YWwgU0EiLCJhdWQiOiJ1bml0eS50YnhhcGlzLmNvbSIsImlhdCI6MTc3NjUxMzc1NSwiZXhwIjoxNzc2Njg2NTU1LCJjb3VudHJ5IjoiQ08iLCJkZXZpY2VDb3VudHJ5IjoiQ08iLCJsYW5ndWFnZSI6ImVzIiwiY2xpZW50IjoiNmE1NjFkMDQ4NzI4ZGI3Yzc4NmI1M2IwOTQxZDBkZDkiLCJkZXZpY2UiOiI5N2RhNGFlZmU0OWEzZDMyZGQyZWE3YTI3NmFhMjdhODcxMDk1MGM1Iiwic3Vic2NyaWJlciI6IjY3OGU2MTRkODFjYzNhMDAwODY1ZTM4MyIsImluZGV4IjoiNjc2MWI2YWI1NWFkZWYwMjJlZTk3ZDE2IiwiY3VzdG9tZXIiOiI2OWQ1YTZhYmM3ODhlYWZlYmU0ZWU3MWIiLCJwcm9maWxlIjoiNjlkNWE2Y2JmMWNkMDMxMzAwZTBiZWYxIiwibWF4UmF0aW5nIjozfQ.WjCMtXNQTfUMOFmwXCO5DtpO-biSGw370JNt6rOxW-w

ENV TBXAPIS_CONTENT_ID=692dc3d7ddd30f329e90a4cc

ENV DECRYPTION_KEY=a18d219ded2ebfa8f41db4a110324a9e

HEALTHCHECK --interval=15s --timeout=5s --start-period=15s --retries=3 \
    CMD wget -qO- http://127.0.0.1:3001/health || exit 1

CMD ["restream_dash"]
