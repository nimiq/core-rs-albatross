FROM ubuntu:22.04

# Install dependencies in a single layer to reduce the number of image layers.
RUN apt-get update && \
    apt-get --no-install-recommends -y install libssl3 tini && \
    rm -rf /var/lib/apt/lists/*

# Run as an unprivileged user, combining commands to reduce layers.
RUN groupadd --system --gid 1001 nimiq && \
    adduser --system --home /home/nimiq --uid 1001 --gid 1001 nimiq

# Switch to the unprivileged user and set working directory in one layer.
USER nimiq
WORKDIR /home/nimiq

# Create the configuration directory
RUN mkdir -p /home/nimiq/.nimiq

# Copy configuration file and binaries in one command to improve caching.
COPY ./lib/src/config/config_file/client.example.toml /home/nimiq/.nimiq/client.toml
COPY ./target/release/nimiq-client \
     ./target/release/nimiq-bls \
     ./target/release/nimiq-address \
     ./target/release/nimiq-rpc /usr/local/bin/

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
