FROM ubuntu:21.04

# Install dependencies.
RUN apt-get update \
  && apt-get --no-install-recommends -y install libssl1.1 tini \
  && rm -rf /var/lib/apt/lists/*

# Run as unprivileged user.
RUN groupadd --system --gid 1001 nimiq \
    && adduser --system --home /home/nimiq --uid 1001 --gid 1001 nimiq
USER nimiq

WORKDIR /home/nimiq

# Create .nimiq, so that it has the right permissions. docker-compose might want to create this
# because we want to use this as a volume, but we don't want this directory to be owned by root.
RUN mkdir -p /home/nimiq/.nimiq
VOLUME /home/nimiq/.nimiq

# Copy necessary files from host environment
COPY ./scripts/docker_*.sh /home/nimiq/

ARG BUILD=debug
COPY ./target/${BUILD}/nimiq-client /usr/local/bin/nimiq-client

EXPOSE 8443/tcp 8648/tcp

ENTRYPOINT [ "/usr/bin/tini", "--", "/home/nimiq/docker_run.sh" ]


# https://github.com/opencontainers/image-spec/blob/master/annotations.md
LABEL \
  org.opencontainers.image.title="Nimiq core-rs-albatross" \
  org.opencontainers.image.description="Rust implementation of the Nimiq Blockchain Core Albatross Branch (Buildkit Ubuntu image)" \
  org.opencontainers.image.url="https://github.com/nimiq/core-rs-albatross" \
  org.opencontainers.image.vendor="Nimiq Foundation" \
  org.opencontainers.image.licenses="Apache-2.0"
