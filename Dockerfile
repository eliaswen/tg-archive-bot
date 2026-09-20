FROM rust:1.97-bookworm AS builder
WORKDIR /usr/src/tg-archive-bot
COPY . .
RUN SQLX_OFFLINE=true cargo install --locked --path .

FROM debian:bookworm-slim AS cpu-runtime
RUN apt-get update && apt-get install -y ca-certificates curl libgomp1 tar && \
    curl -fsSL https://github.com/microsoft/onnxruntime/releases/download/v1.28.0/onnxruntime-linux-x64-1.28.0.tgz -o /tmp/ort.tgz && \
    tar -xzf /tmp/ort.tgz -C /opt && \
    ln -s /opt/onnxruntime-linux-x64-1.28.0/lib/libonnxruntime.so /usr/local/lib/libonnxruntime.so && \
    rm /tmp/ort.tgz && rm -rf /var/lib/apt/lists/*
ENV ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so
COPY --from=builder /usr/local/cargo/bin/tg-archive-bot /usr/local/bin/tg-archive-bot
EXPOSE 3000
ENTRYPOINT ["tg-archive-bot"]
CMD ["bot"]

FROM nvidia/cuda:12.8.1-cudnn-runtime-ubuntu24.04 AS cuda-runtime
RUN apt-get update && apt-get install -y ca-certificates curl libgomp1 tar && \
    curl -fsSL https://github.com/microsoft/onnxruntime/releases/download/v1.28.0/onnxruntime-linux-x64-gpu_cuda12-1.28.0.tgz -o /tmp/ort.tgz && \
    tar -xzf /tmp/ort.tgz -C /opt && \
    ln -s /opt/onnxruntime-linux-x64-gpu_cuda12-1.28.0/lib/libonnxruntime.so /usr/local/lib/libonnxruntime.so && \
    rm /tmp/ort.tgz && rm -rf /var/lib/apt/lists/*
ENV ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so
COPY --from=builder /usr/local/cargo/bin/tg-archive-bot /usr/local/bin/tg-archive-bot
EXPOSE 3000
ENTRYPOINT ["tg-archive-bot"]
CMD ["ml"]

FROM cpu-runtime AS default
