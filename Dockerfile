FROM rust:latest AS builder
WORKDIR /usr/src/tg-archive-bot
COPY . .
RUN SQLX_OFFLINE=true cargo install --path .

FROM debian:trixie
RUN apt-get update && apt-get install -y ca-certificates curl && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/tg-archive-bot /usr/local/bin/tg-archive-bot
EXPOSE 3000
ENTRYPOINT ["tg-archive-bot"]
CMD ["bot"]
