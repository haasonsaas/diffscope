# Frontend build stage
FROM node:20-alpine AS frontend

WORKDIR /app/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ .
RUN npm run build

# Rust build stage
FROM rust:alpine AS builder

RUN apk add --no-cache musl-dev

WORKDIR /app

# Cache dependencies
COPY Cargo.toml Cargo.lock ./
COPY build.rs ./build.rs
COPY migrations ./migrations
RUN mkdir src && echo 'fn main() {}' > src/main.rs && \
    mkdir -p web/dist && \
    cargo build --release && \
    rm -rf src

# Copy frontend build output (embedded via rust-embed)
COPY --from=frontend /app/web/dist ./web/dist

# Build actual binary
COPY src ./src
RUN touch src/main.rs && cargo build --release

# Runtime stage
FROM alpine:3.19

RUN apk add --no-cache ca-certificates git

RUN addgroup -g 1000 diffscope && \
    adduser -u 1000 -G diffscope -h /home/diffscope -s /bin/sh -D diffscope && \
    mkdir -p /home/diffscope/.local/share/diffscope \
             /home/diffscope/.diffscope && \
    chown -R diffscope:diffscope /home/diffscope

COPY --from=builder /app/target/release/diffscope /usr/local/bin/diffscope

USER diffscope
WORKDIR /home/diffscope

EXPOSE 3000

ENTRYPOINT ["diffscope"]
CMD ["serve", "--host", "0.0.0.0", "--port", "3000"]
