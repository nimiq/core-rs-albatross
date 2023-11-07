FROM ubuntu:22.04

# Install dependencies.
RUN apt-get update \
    && apt-get --no-install-recommends -y install libssl3 tini \
    && rm -rf /var/lib/apt/lists/*

# Run as unprivileged user.
RUN groupadd --system --gid 1001 nimiq \
    && adduser --system --home /home/nimiq --uid 1001 --gid 1001 nimiq
USER nimiq

# Change homedir to nimiq
WORKDIR /home/nimiq

# Create nimiq directory for configuration
RUN mkdir -p /home/nimiq/.nimiq

# Set default config can be overwritten by mounting
COPY ./client.toml /home/nimiq/.nimiq

COPY ./target/release/nimiq-client /usr/local/bin/nimiq-client
COPY ./target/release/nimiq-bls /usr/local/bin/nimiq-bls
COPY ./target/release/nimiq-address /usr/local/bin/nimiq-address
COPY ./target/release/nimiq-rpc /usr/local/bin/nimiq-rpc

# Expose the incoming connections port
EXPOSE 8443

# Expose RPC port
EXPOSE 8648

# Expose metrics port
EXPOSE 9100

# Run CMD so we can use other bin
CMD [ "/usr/bin/tini", "--", "nimiq-client" ]

LABEL \
    org.opencontainers.image.title="Nimiq core-rs-albatross" \
    org.opencontainers.image.description="Rust implementation of the Nimiq Blockchain Core Albatross Branch (Ubuntu image)" \
    org.opencontainers.image.url="https://github.com/nimiq/core-rs-albatross" \
    org.opencontainers.image.vendor="Nimiq Foundation" \
    org.opencontainers.image.licenses="Apache-2.0" \
    org.opencontainers.image.source="https://github.com/nimiq/core-rs-albatross/"

