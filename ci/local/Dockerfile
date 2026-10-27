# Toolchain of the CI deb job: debian:bookworm, rustup, cargo-deb and the
# aarch64 cross gcc.
FROM debian:bookworm
RUN apt-get update && apt-get install -y --no-install-recommends build-essential ca-certificates curl git \
      gcc-aarch64-linux-gnu libc6-dev-arm64-cross binutils openssh-client openssh-server systemd python3 \
    && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal \
      --target x86_64-unknown-linux-gnu --target aarch64-unknown-linux-gnu \
    && rustup component add clippy rustfmt \
    && cargo install cargo-deb --locked \
    && chmod -R a+rwX /opt/cargo /opt/rustup
