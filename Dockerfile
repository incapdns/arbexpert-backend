# Etapa 1: Compilar o projeto
FROM rust:1.88-trixie as builder

WORKDIR /appr
COPY . .

RUN cargo build --release

FROM debian:trixie-slim

COPY --from=builder /app/target/release/arbexpert /usr/local/bin/arbexpert

ENTRYPOINT ["/usr/local/bin/arbexpert"]
