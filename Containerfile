# Multi-stage production container for AndroGrid Control Plane Orchestrator (Podman Containerfile)
# Traces to: AG-SPEC-2026-V2 Section 2.3, [INV-002], [NFR-SEC-001], [NFR-PERF-001]

# Stage 1: Build binary using official Rust 1.82 toolchain
FROM docker.io/library/rust:1.82-bookworm AS builder

WORKDIR /usr/src/androgrid-backend

# Copy dependency manifests and build scripts
COPY Cargo.toml Cargo.lock build.rs ./
COPY proto ./proto
COPY src ./src

# Compile production binaries with release profile
RUN cargo build --release --bin androgrid-backend --bin androgrid-cli

# Stage 2: Hardened, unprivileged runtime image
FROM docker.io/library/debian:bookworm-slim AS runtime

# Install TLS root certificates
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Run as dedicated unprivileged system user
RUN groupadd -g 10001 androgrid && \
    useradd -u 10001 -g androgrid -s /bin/false -M androgrid

WORKDIR /app

# Copy binaries from builder stage
COPY --from=builder /usr/src/androgrid-backend/target/release/androgrid-backend /usr/local/bin/androgrid-backend
COPY --from=builder /usr/src/androgrid-backend/target/release/androgrid-cli /usr/local/bin/androgrid-cli

# Ensure binaries and working directory are owned by unprivileged user
RUN chown -R androgrid:androgrid /app /usr/local/bin/androgrid-backend /usr/local/bin/androgrid-cli

USER androgrid

# Expose default gRPC port
EXPOSE 50051

# Default environment settings
ENV ANDROGRID_LISTEN_ADDR=0.0.0.0:50051
ENV RUST_LOG=info

ENTRYPOINT ["/usr/local/bin/androgrid-backend"]
