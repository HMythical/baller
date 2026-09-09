# Cross-compiles B.A.L.L.E.R. for Windows (x86_64-pc-windows-gnu) from a Linux container.
FROM debian:bookworm-slim AS build

RUN apt-get update && apt-get install -y --no-install-recommends \
    curl \
    ca-certificates \
    build-essential \
    mingw-w64 \
    binutils-mingw-w64-x86-64 \
    && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH \
    RUSTFLAGS="-C linker=x86_64-w64-mingw32-gcc"

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable \
    && rustup target add x86_64-pc-windows-gnu

WORKDIR /src
COPY . .

# Label the binary with the requested release version (no v prefix).
ARG BALLER_VERSION=0.1.0
RUN sed -i "s/^version = .*/version = \"$BALLER_VERSION\"/" Cargo.toml

RUN cargo build --release --target x86_64-pc-windows-gnu

RUN mkdir -p /dist \
    && cp target/x86_64-pc-windows-gnu/release/baller.exe /dist/ \
    && x86_64-w64-mingw32-objdump -f target/x86_64-pc-windows-gnu/release/baller.exe | head -6

FROM scratch AS artifact
COPY --from=build /dist/ /