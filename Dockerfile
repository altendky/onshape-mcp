# Stage 1: Build the binary
FROM rust:1.89-slim AS builder

WORKDIR /build

# Install build dependencies (pkg-config needed for some crates)
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace manifests first for dependency caching
COPY Cargo.toml Cargo.lock ./
COPY crates/onshape-client-core/Cargo.toml crates/onshape-client-core/
COPY crates/onshape-client-io/Cargo.toml crates/onshape-client-io/
COPY crates/onshape-mcp-core/Cargo.toml crates/onshape-mcp-core/
COPY crates/onshape-mcp-io/Cargo.toml crates/onshape-mcp-io/
COPY crates/onshape-mcp-resources/Cargo.toml crates/onshape-mcp-resources/
COPY crates/onshape-mcp/Cargo.toml crates/onshape-mcp/

# Create dummy source files so cargo can resolve dependencies
RUN mkdir -p crates/onshape-client-core/src && echo "" > crates/onshape-client-core/src/lib.rs \
    && mkdir -p crates/onshape-client-io/src && echo "" > crates/onshape-client-io/src/lib.rs \
    && mkdir -p crates/onshape-mcp-core/src && echo "" > crates/onshape-mcp-core/src/lib.rs \
    && mkdir -p crates/onshape-mcp-io/src && echo "" > crates/onshape-mcp-io/src/lib.rs \
    && mkdir -p crates/onshape-mcp-resources/src && echo "" > crates/onshape-mcp-resources/src/lib.rs \
    && mkdir -p crates/onshape-mcp/src && echo "fn main() {}" > crates/onshape-mcp/src/main.rs

# Pre-build dependencies (cached unless Cargo.toml/Cargo.lock change)
RUN cargo build --release --package onshape-mcp 2>/dev/null || true

# Copy full source and build for real
COPY crates/ crates/
RUN touch crates/*/src/*.rs crates/*/src/**/*.rs 2>/dev/null; \
    cargo build --release --package onshape-mcp

# Stage 2: Minimal runtime image
FROM debian:bookworm-slim

# Install CA certificates (needed for HTTPS to Onshape API)
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/onshape-mcp /usr/local/bin/onshape-mcp

EXPOSE 8080

ENTRYPOINT ["onshape-mcp"]
CMD ["http"]
