# Build stage
FROM rust:alpine AS builder

RUN apk add --no-cache musl-dev

# Install the musl target
RUN rustup target add x86_64-unknown-linux-musl

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --target x86_64-unknown-linux-musl
RUN strip target/x86_64-unknown-linux-musl/release/diffscope

# Runtime stage
FROM alpine:3.19

RUN apk add --no-cache ca-certificates

COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/diffscope /usr/local/bin/diffscope

ENTRYPOINT ["diffscope"]
CMD ["--help"]