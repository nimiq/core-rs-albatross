"""

    # Albatross DevNet scripts

    ## Usage

    1. Run `devnet_create.py NUM_VALIDATORS`. This will create keys and configurations for multiple validator nodes.
    2. Copy genesis config from `/tmp/nimiq-devnet-RANDOM/dev-albatross.toml` to `core-rs/network-primitives/src/genesis/dev-albatross.toml`.
    3. Build core-rs: `cargo build`
    4. Run seed node. Run a node (not as validator) at `127.0.0.1:8443`
    5. Run `devnet_run.py PATH` (with `PATH=/tmp/nimiq-devnet-RANDOM`). This will start the validators.

    ## Notes

    - The path to the `core-rs/target/debug` source code must be set in `devnet_create.py` and `devnet_run.py` in the `TARGET` variable.
    - Logs files of the validators are in in `/tmp/nimiq-devnet-RANDOM/validatorNUM/nimiq-client.log`

"""


from binascii import unhexlify
from pathlib import Path
from tempfile import mkdtemp
import sh
import json
from datetime import datetime
from sys import argv


try:
    num_validators = int(argv[1])
except (IndexError, ValueError):
    print("Usage: {} NUM_VALIDATORS".format(argv[0]))
    exit(1)


#tmp = Path(mkdtemp(prefix="nimiq_devnet-"))
tmp = Path("/tmp/nimiq-devnet")


TARGET = Path("/home/janosch/nimiq/dev/core-rs-albatross/target/debug")
nimiq_address = sh.Command(str(TARGET / "nimiq-address"))
nimiq_bls = sh.Command(str(TARGET / "nimiq-bls"))
nimiq_client = sh.Command(str(TARGET / "nimiq-client"))

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


def create_validator(path, i):
    path.mkdir(parents=True)

    # create BLS keypair
    validator_key = create_bls_keypair()
    with (path / "validator_key.dat").open("wb") as f:
        f.write(unhexlify(validator_key["private_key"]))

    # create staking (and reward) address
    staker_address = create_nimiq_address()
    reward_address = create_nimiq_address()

    # write config
    with (path / "client.toml").open("wt") as f:
        f.write("""
peer-key-file = "{path}/peer_key.dat"

[network]
host = "{hostname}"
port = {port}
seed_nodes = [
    {{ uri = "ws://127.0.0.1:8443/8a90ec195a8cba3643509171cc59095cb3de6bd65f5aa7a2f84b63805989566f" }}
]

[consensus]
network = "dev-albatross"

[log]
level = "trace"
file = "{path}/nimiq-client.log"

[database]
path = "{path}/"

[validator]
type = "validator"
block_delay = 250
key_file = "{path}/validator_key.dat"
    """.format(
            hostname="127.0.1.{}".format(i + 1),
            port=str(8500 + i),
            path=str(path)
        ))

    return {
        "validator_key": validator_key,
        "staker_address": staker_address,
        "reward_address": reward_address,
        "path": str(path)
    }

print("Writing devnet to: {}".format(tmp))
print("Creating validators...")
validators = []
for i in range(num_validators):
    validator = create_validator(tmp / "validator{:d}".format(i), i)
    validators.append(validator)
    print("Created validator: {}..".format(validator["validator_key"]["public_key"][0:16]))

print("Writing genesis config")
with (tmp / "dev-albatross.toml").open("wt") as f:
    f.write("""
name = "dev-albatross"
seed_message = "Albatross DevNet"
signing_key = "230cf5070e9362108e3549360b84be23826c23839124b917629fb525db3baece"
timestamp="{timestamp}"
    """.format(
        #timestamp=datetime.utcnow().isoformat()
        timestamp="2019-05-10T23:56:52.776772644+00:00"
    ))
    for validator in validators:
        f.write("""
[[stakes]]
staker_address = "{staker_address}"
reward_address = "{reward_address}"
validator_key = "{validator_key}"
balance = 100000
        """.format(
            staker_address=validator["staker_address"]["address"],
            reward_address=validator["reward_address"]["address"],
            validator_key=validator["validator_key"]["public_key"]
        ))

print("Writing configuration")
with (tmp / "validators.json").open("wt") as f:
    json.dump(validators, f)
