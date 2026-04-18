FROM rust:1.94.1-bookworm

RUN apt-get update && apt-get install -y libudev-dev pkg-config && rm -rf /var/lib/apt/lists/*
RUN rustup target install thumbv6m-none-eabi
RUN cargo install flip-link --version 0.1.12
RUN rustup component add rustfmt
RUN rustup component add llvm-tools
RUN cargo install cargo-binutils
RUN cargo install elf2uf2-rs --locked
