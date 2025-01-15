FROM ubuntu:24.04 AS build

# Install build dependencies in a single layer to reduce the number of image layers.
RUN apt-get update && \
    apt-get --no-install-recommends -y install ca-certificates clang cmake curl git libssl-dev pkg-config && \
    rm -rf /var/lib/apt/lists/*

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# Compile the code
WORKDIR /root
COPY ./ core-rs-albatross
RUN cd core-rs-albatross && \
    cargo build --release --bin nimiq-client --bin nimiq-bls --bin nimiq-address --bin nimiq-rpc

FROM ubuntu:24.04

# Install dependencies in a single layer to reduce the number of image layers.
RUN apt-get update && \
    apt-get --no-install-recommends -y install libssl3 tini curl && \
    rm -rf /var/lib/apt/lists/*

# Run as an unprivileged user, combining commands to reduce layers.
RUN groupadd --system --gid 1001 nimiq && \
    useradd --system --home /home/nimiq --uid 1001 --gid 1001 nimiq

# Switch to the unprivileged user and set working directory in one layer.
USER nimiq
WORKDIR /home/nimiq

# Create the configuration directory
RUN mkdir -p /home/nimiq/.nimiq

# Copy configuration file and binaries in one command to improve caching.
COPY --from=build /root/core-rs-albatross/lib/src/config/config_file/client.example.toml /home/nimiq/.nimiq/client.toml
COPY --from=build /root/core-rs-albatross/target/release/nimiq-client \
     /root/core-rs-albatross/target/release/nimiq-bls \
     /root/core-rs-albatross/target/release/nimiq-address \
     /root/core-rs-albatross/target/release/nimiq-rpc /usr/local/bin/

# Expose the necessary ports
EXPOSE 8443 8648 9100

# Use CMD to run the nimiq-client with tini as an init system.
CMD [ "/usr/bin/tini", "--", "nimiq-client" ]

# Labels for image metadata.
LABEL org.opencontainers.image.title="Nimiq core-rs-albatross" \
      org.opencontainers.image.description="Rust implementation of the Nimiq Blockchain Core Albatross Branch (Ubuntu image)" \
      org.opencontainers.image.url="https://github.com/nimiq/core-rs-albatross" \
      org.opencontainers.image.vendor="Nimiq Foundation" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.source="https://github.com/nimiq/core-rs-albatross/"
