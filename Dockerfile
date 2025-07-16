# Etapa 1: Compilar o projeto
FROM rust:1.88-trixie as builder

RUN apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    libclang-dev \
    cmake \
    curl \
    protobuf-compiler

WORKDIR /app

COPY Cargo.toml Cargo.lock ./

RUN mkdir src && echo "fn main() {}" > src/main.rs

RUN cargo build --release || true

COPY . .

RUN cargo build --release

FROM debian:trixie-slim

COPY --from=builder /app/target/release/arbexpert /usr/local/bin/arbexpert

ENTRYPOINT ["/usr/local/bin/arbexpert"]
