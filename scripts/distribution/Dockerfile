FROM python:3.12-bookworm@sha256:21206b8f02efda6d737e402e1c716b650ad30605843a36e4bcb55463a7ebdf37
ARG RUST_VERSION=1.97.1
ARG NODE_VERSION=22.21.1
ARG TAURI_CLI_VERSION=2.11.4
ARG CARGO_DENY_VERSION=0.19.9
ENV DEBIAN_FRONTEND=noninteractive PATH=/root/.cargo/bin:/usr/local/bin:$PATH
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential ca-certificates curl git python3 pkg-config file binutils \
    libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev \
    libxdo-dev libssl-dev libfuse2 patchelf squashfs-tools zlib1g-dev \
    tcl8.6-dev tcl8.6 xvfb xauth lsof ripgrep libnss3 libxss1 libasound2 fonts-liberation libgbm1 && rm -rf /var/lib/apt/lists/*
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup.sh \
    && sh /tmp/rustup.sh -y --profile minimal --default-toolchain "$RUST_VERSION" \
    && rustup component add rustfmt clippy && rm /tmp/rustup.sh
RUN arch=$(dpkg --print-architecture) && case "$arch" in amd64) arch=x64;; arm64) arch=arm64;; esac \
    && curl -fsSLO "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-${arch}.tar.xz" \
    && curl -fsSL "https://nodejs.org/dist/v${NODE_VERSION}/SHASUMS256.txt" -o /tmp/node-sha \
    && awk -v name="node-v${NODE_VERSION}-linux-${arch}.tar.xz" '$2 == name' /tmp/node-sha | sha256sum -c - \
    && tar -xJf "node-v${NODE_VERSION}-linux-${arch}.tar.xz" -C /usr/local --strip-components=1 \
    && rm "node-v${NODE_VERSION}-linux-${arch}.tar.xz" /tmp/node-sha
RUN cargo install tauri-cli --version "$TAURI_CLI_VERSION" --locked
ENV APPIMAGE_EXTRACT_AND_RUN=1 TCL8_6_PREFIX=/usr TCLD_TCLSH86=/usr/bin/tclsh8.6
RUN cargo install cargo-deny --version "$CARGO_DENY_VERSION" --locked
WORKDIR /work
