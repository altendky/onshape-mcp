# Stage 1: Build the binary
FROM rust:1.98-slim AS builder

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

# onshape-mcp-resources has a build.rs that panics if resources/ is missing.
# Copy the build.rs and create an empty resources dir for the dependency cache step.
COPY crates/onshape-mcp-resources/build.rs crates/onshape-mcp-resources/

# Create dummy source files so cargo can resolve dependencies.
# The empty resources/ dir satisfies the build.rs existence check.
RUN mkdir -p crates/onshape-client-core/src && echo "" > crates/onshape-client-core/src/lib.rs \
    && mkdir -p crates/onshape-client-io/src && echo "" > crates/onshape-client-io/src/lib.rs \
    && mkdir -p crates/onshape-mcp-core/src && echo "" > crates/onshape-mcp-core/src/lib.rs \
    && mkdir -p crates/onshape-mcp-io/src && echo "" > crates/onshape-mcp-io/src/lib.rs \
    && mkdir -p crates/onshape-mcp-resources/src && echo "" > crates/onshape-mcp-resources/src/lib.rs \
    && mkdir -p crates/onshape-mcp-resources/resources \
    && mkdir -p crates/onshape-mcp/src && echo "fn main() {}" > crates/onshape-mcp/src/main.rs

# Pre-build dependencies (cached unless Cargo.toml/Cargo.lock change)
RUN cargo build --release --package onshape-mcp 2>/dev/null || true

# Copy full source. First remove the dummy resources dir so the COPY doesn't
# conflict with the symlink in the source tree.
RUN rm -rf crates/onshape-mcp-resources/resources
COPY crates/ crates/
# The resources/ symlink won't resolve inside Docker — replace it with the
# real directory contents.
RUN rm -f crates/onshape-mcp-resources/resources
COPY docs/src/mcp-resources/ crates/onshape-mcp-resources/resources/
RUN touch crates/*/src/*.rs crates/*/src/**/*.rs 2>/dev/null; \
    cargo build --release --package onshape-mcp

# Stage 2: Minimal runtime image
# Must match the builder's Debian version (rust:1.89-slim is based on trixie)
# to avoid glibc version mismatches.
FROM debian:trixie-slim

# Install CA certificates (HTTPS) and curl (HEALTHCHECK)
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user for container security
RUN useradd --create-home --shell /bin/bash appuser

COPY --from=builder /build/target/release/onshape-mcp /usr/local/bin/onshape-mcp

USER appuser

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

ENTRYPOINT ["onshape-mcp"]
CMD ["http"]
