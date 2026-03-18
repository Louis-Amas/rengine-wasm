FROM rust:slim as builder

# Install build deps
RUN apt-get update && apt-get install -y \
    build-essential pkg-config libssl-dev curl unzip

# Copy local DuckDB binaries (from deps/duckdb)
COPY deps/duckdb /deps/duckdb
RUN install -D -m755 /deps/duckdb/libduckdb.so /usr/local/lib/libduckdb.so && \
    ldconfig

ENV LD_LIBRARY_PATH="/usr/local/lib"

WORKDIR /app
COPY . .

RUN RUSTFLAGS="-L /usr/local/lib" cargo build --release -p trader

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    libssl3 ca-certificates unzip curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/lib/libduckdb.so /usr/local/lib/libduckdb.so
RUN ldconfig

ENV LD_LIBRARY_PATH="/usr/local/lib"

COPY --from=builder /app/target/release/trader /usr/local/bin/trader

ENTRYPOINT ["/usr/local/bin/trader"]

