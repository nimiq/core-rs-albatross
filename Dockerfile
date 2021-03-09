FROM ubuntu:20.04
RUN apt-get update \
 && apt-get install -y libssl1.1 \
 && rm -rf /var/lib/apt

# Run as unprivileged user.
RUN adduser --disabled-password --home /home/nimiq --shell /bin/bash --uid 1001 nimiq
USER nimiq

COPY ./scripts/docker_*.sh /home/nimiq/
COPY ./target/debug/nimiq-client /usr/local/bin/
WORKDIR /home/nimiq

ENV NIMIQ_NETWORK=dev-albatross \
    NIMIQ_LOG_LEVEL=debug \
    NIMIQ_VALIDATOR=none \
    VALIDATOR_BLOCK_DELAY=250 \
    RPC_ENABLED=false

EXPOSE 8443/tcp 8648/tcp

VOLUME [ "/home/nimiq/database" ]

ENTRYPOINT [ "/bin/bash" ]
CMD [ "/home/nimiq/docker_run.sh" ]
