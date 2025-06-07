# Build stage
FROM rust:alpine AS builder

RUN apk add --no-cache musl-dev

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Build for the native architecture
RUN cargo build --release
RUN strip target/release/diffscope

# Runtime stage
FROM alpine:3.19

RUN apk add --no-cache ca-certificates

COPY --from=builder /app/target/release/diffscope /usr/local/bin/diffscope

ENTRYPOINT ["diffscope"]
CMD ["--help"]