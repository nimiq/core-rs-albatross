"""

    # Albatross DevNet scripts

    ## Usage

    1. Run `devnet_create.py NUM_VALIDATORS`. This will create keys and configurations for multiple validator nodes.
    2. Copy genesis config from `/tmp/nimiq-devnet-RANDOM/dev-albatross.toml` to `core-rs/genesis/src/genesis/dev-albatross.toml`.
    3. Build core-rs: `cargo build`
    4. Run seed node. Run a node (not as validator) at `127.0.0.1:8443`
    5. Run `devnet_run.py PATH` (with `PATH=/tmp/nimiq-devnet-RANDOM`). This will start the validators.

    ## Notes

    - The path to the `core-rs/target/debug` source code must be set in `devnet_create.py` and `devnet_run.py` in the `TARGET` variable.
    - Logs files of the validators are in in `/tmp/nimiq-devnet-RANDOM/validatorNUM/nimiq-client.log`

"""


from binascii import unhexlify
from pathlib import Path
import sh
import json
from sys import argv


try:
    num_validators = int(argv[1])
except (IndexError, ValueError):
    print("Usage: {} NUM_VALIDATORS [OUTPUT]".format(argv[0]))
    exit(1)

try:
    output = Path(argv[2])
except IndexError:
    output = Path("/tmp/nimiq-devnet")

target = Path.cwd() / "target" / "debug"

nimiq_address = sh.Command(str(target / "nimiq-address"))
nimiq_bls = sh.Command(str(target / "nimiq-bls"))


def create_bls_keypair():
    lines = []
    for l in nimiq_bls():
        l = l.strip()
        if l and not l.startswith("#"):
            lines.append(l)
    return {
        "public_key": lines[0],
        "private_key": lines[1]
    }


def create_nimiq_address():
    lines = []
    for i, l in enumerate(nimiq_address()):
        lines.append(l.split(":")[1].strip())
    return {
        "address": lines[0],
        "address_raw": lines[1],
        "public_key": lines[2],
        "private_key": lines[3]
    }


def create_seed(path, i):
    path.mkdir(parents=True, exist_ok=True)

    # write config
    with (path / "client.toml").open("wt") as f:
        f.write("""[network]
peer_key_file = "{path}/peer_key.dat"
listen_addresses = [
	"/ip4/127.0.0.1/tcp/9100/ws",
]

[consensus]
network = "dev-albatross"
min_peers = 1

[database]
path = "{path}"

[log]
level = "trace"
timestamps = true

[log.tags]
libp2p_swarm = "debug"
lock_api = "trace"
""".format(
            path="temp-state/dev/seed",
        ))


def create_spammer(path, i):
    path.mkdir(parents=True, exist_ok=True)

    # write config
    with (path / "client.toml").open("wt") as f:
        f.write("""[network]
peer_key_file = "{path}/peer_key.dat"
listen_addresses = [
	"/ip4/127.0.0.1/tcp/9999/ws",
]
seed_nodes = [
	{{ address = "/ip4/127.0.0.1/tcp/9101/ws" }} ,
    {{ address = "/ip4/127.0.0.1/tcp/9102/ws" }}
]

[consensus]
network = "dev-albatross"
min_peers = 1

[database]
path = "{path}"

[log]
level = "trace"
timestamps = true

[log.tags]
libp2p_swarm = "debug"
lock_api = "trace"

[validator]
validator_address = "NQ07 0000 0000 0000 0000 0000 0000 0000 0000"
signing_key_file = "{path}/signing_key.dat"
voting_key_file = "{path}/voting_key.dat"
fee_key_file = "{path}/fee_key.dat"
""".format(
            path="temp-state/dev/spammer",
        ))


def create_validator(path, i):
    path.mkdir(parents=True, exist_ok=True)

    # create voting (BLS) keypair
    voting_key = create_bls_keypair()

    # create signing (Schnorr) keypair
    signing_key = create_nimiq_address()

    # create staking (and reward) address
    validator_address = create_nimiq_address()
    reward_address = create_nimiq_address()

    # write parameters for ansible
    with (path / "validator{:d}.yml".format(i+1)).open("wt") as f:
        f.write("""---
validator_address: "{validator_address}"
voting_key: "{voting_key}"
signing_key: "{signing_key}"
fee_key: "{fee_key}"
""".format(
            validator_address=validator_address["address"],
            voting_key=voting_key["private_key"],
            signing_key=signing_key["private_key"],
            fee_key=reward_address["private_key"]
        ))

    # write config
    with (path / "client.toml").open("wt") as f:
        f.write("""[network]
peer_key_file = "{path}/peer_key.dat"
listen_addresses = [
	"/ip4/127.0.0.1/tcp/{port}/ws",
]
seed_nodes = [
	{{ address = "/ip4/127.0.0.1/tcp/9100/ws" }}
]

[consensus]
network = "dev-albatross"
min_peers = 1

[database]
path = "{path}"

[log]
level = "trace"
timestamps = true

[log.tags]
libp2p_swarm = "debug"
lock_api = "trace"

[validator]
validator_address = "{validator_address}"
signing_key_file = "{path}/signing_key.dat"
signing_key = "{signing_key}"
voting_key_file = "{path}/voting_key.dat"
voting_key = "{voting_key}"
fee_key_file = "{path}/fee_key.dat"
fee_key = "{fee_key}"
""".format(
            port=str(9101 + i),
            path="temp-state/dev/{}".format(i+1),  # str(path),
            validator_address=validator_address["address"],
            voting_key=voting_key["private_key"],
            signing_key=signing_key["private_key"],
            fee_key=reward_address["private_key"]
        ))

    return {
        "voting_key": voting_key,
        "validator_address": validator_address,
        "signing_key": signing_key,
        "reward_address": reward_address,
        "path": str(path)
    }


print("Writing devnet to: {}".format(output))
print("Creating validators...")
validators = []
for i in range(num_validators):
    validator = create_validator(output / "validator{:d}".format(i+1), i)
    validators.append(validator)
    print("Created validator: {}..".format(
        validator["voting_key"]["public_key"][0:16]))

# Create seed node configuration
create_seed(output / "seed", 1)
print("Created seed node configuration")

# Create spammer node configuration
create_spammer(output / "spammer", 1)
print("Created spammer configuration")

# Genesis configuration
print("Writing genesis config")
with (output / "dev-albatross.toml").open("wt") as f:
    f.write("""name = "dev-albatross"
seed_message = "Albatross DevNet"
timestamp = "{timestamp}"
vrf_seed = "e8c7f2f3935da9ca39419aa7d2cc90817245f75e58cc543f2b9478766308e8a50fffccb09e2df3546f5a0c0059d73a506c48fa2b546f15b511d0f7a63f0ee20cd510a87f520e26478bb687ca31a08db8b02921f9a22e32a790c07f16dbdf4501"
""".format(
        # timestamp=datetime.utcnow().isoformat()
        timestamp="2021-07-15T00:00:00.000+00:00"
    ))
    for validator in validators:
        f.write("""
[[validators]]
validator_address = "{validator_address}"
signing_key = "{signing_key}"
voting_key = "{voting_key}"
reward_address = "{reward_address}"
""".format(
            validator_address=validator["validator_address"]["address"],
            signing_key=validator["signing_key"]["public_key"],
            voting_key=validator["voting_key"]["public_key"],
            reward_address=validator["reward_address"]["address"]
        ))
    for validator in validators:
        f.write("""
[[stakers]]
staker_address = "{reward_address}"
balance = 1_000_000
delegation = "{validator_address}"
""".format(
            reward_address=validator["reward_address"]["address"],
            validator_address=validator["validator_address"]["address"]
        ))
    f.write("""
[[accounts]]
address = "NQ37 7C3V VMN8 FRPN FXS9 PLAG JMRE 8SC6 KUSQ"
balance = 10_000_000_00000
""")

# Docker compose configuration
print("Writing docker compose config")
with (output / "docker-compose.yml").open("wt") as f:
    f.write("""version: "3.5"

networks:
  devnet:
    name: ${NETWORK_NAME:?err}
    driver: bridge
    ipam:
      driver: default
      config:
        - subnet: 7.0.0.0/24
""")
    # Seed node
    f.write("""
services:
  seed0:
    image: core:latest
    environment:
      - LISTEN_ADDRESSES=/ip4/7.0.0.99/tcp/8443/ws
      - NIMIQ_HOST=seed0.${NETWORK_NAME:?err}
      - NIMIQ_NETWORK=dev-albatross
      - NIMIQ_PEER_KEY_FILE=/home/nimiq/.nimiq/peer_key.dat
      - NIMIQ_INSTANT_INBOUND=true
      - RPC_ENABLED=true
      - RUST_BACKTRACE="1"
      - NIMIQ_LOG_LEVEL=debug
      - NIMIQ_LOG_TIMESTAMPS=true
    networks:
      devnet:
        ipv4_address: 7.0.0.99
    volumes:
      - "seed0:/home/nimiq/.nimiq:rw"
""")

# Writing validator configuration
    for idx, validator in enumerate(validators):
        f.write("""
  seed{validatorid}:
    image: core:latest
    depends_on:
      - seed0
    environment:
      - LISTEN_ADDRESSES=/ip4/{ip}/tcp/8443/ws
      - NIMIQ_HOST=seed{validatorid}.${{NETWORK_NAME:?err}}
      - NIMIQ_NETWORK=dev-albatross
      - NIMIQ_SEED_NODES=/ip4/7.0.0.99/tcp/8443/ws
      - NIMIQ_PEER_KEY_FILE=/home/nimiq/.nimiq/peer_key.dat
      - NIMIQ_INSTANT_INBOUND=true
      - NIMIQ_VALIDATOR=validator
      - VALIDATOR_ADDRESS={validator_address}
      - SIGNING_KEY={signing_key}
      - VOTING_KEY={voting_key}
      - FEE_KEY={fee_key}
      - RPC_ENABLED=false
      - RUST_BACKTRACE="1"
      - NIMIQ_LOG_LEVEL=debug
      - NIMIQ_LOG_TIMESTAMPS=true
    networks:
      devnet:
        ipv4_address: {ip}
    volumes:
      - "seed{validatorid}:/home/nimiq/.nimiq:rw"
""".format(validatorid=str(idx+1),
           ip=str("7.0.0.{}".format(idx+2)),
           validator_address=validator["validator_address"]["address"].replace(
            " ", ""),
           signing_key=validator["signing_key"]["private_key"],
           voting_key=validator["voting_key"]["private_key"],
           fee_key=validator["reward_address"]["private_key"]
           ))

# Spammer node
    f.write("""
  spammer:
    image: spammer:latest
    depends_on:
      - seed0
    environment:
      - LISTEN_ADDRESSES=/ip4/7.0.0.98/tcp/8443/ws
      - NIMIQ_HOST=seed4.${NETWORK_NAME:?err}
      - NIMIQ_NETWORK=dev-albatross
      - NIMIQ_SEED_NODES=/ip4/7.0.0.99/tcp/8443/ws
      - NIMIQ_PEER_KEY_FILE=/home/nimiq/.nimiq/peer_key.dat
      - NIMIQ_INSTANT_INBOUND=true
      - NIMIQ_VALIDATOR=validator
      - VALIDATOR_ADDRESS=NQ0700000000000000000000000000000000
      - RPC_ENABLED=true
      - RUST_BACKTRACE="1"
      - NIMIQ_LOG_LEVEL=info
      - NIMIQ_LOG_TIMESTAMPS=true
    networks:
      devnet:
        ipv4_address: 7.0.0.98
    volumes:
      - "spammer:/home/nimiq/.nimiq:rw"
""")
# Volumes
    f.write("""
volumes:
  spammer:
  seed0:\n""")
    for idx, validator in enumerate(validators):
        f.write("  seed{}:\n".format(idx+1))


print("Writing configuration")
with (output / "validators.json").open("wt") as f:
    json.dump(validators, f)
