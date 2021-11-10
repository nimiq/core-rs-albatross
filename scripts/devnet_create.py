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
    output = Path(argv[3])
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
min_peers = 1

[consensus]
network = "dev-albatross"
min_peers = 1

[database]
path = "{path}"

[log]
level = "debug"
timestamps = true
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
min_peers = 1

[consensus]
network = "dev-albatross"
min_peers = 1

[database]
path = "{path}"

[log]
level = "trace"
timestamps = true

[validator]
validator_address = "NQ07 0000 0000 0000 0000 0000 0000 0000 0000"
    """.format(
            path="temp-state/dev/spammer",
        ))


def create_validator(path, i):
    path.mkdir(parents=True, exist_ok=True)

    # create BLS keypair
    validator_key = create_bls_keypair()
    # with (path / "validator_key.dat").open("wb") as f:
    #    f.write(unhexlify(validator_key["private_key"]))

    # create staking (and reward) address
    validator_address = create_nimiq_address()
    warm_address = create_nimiq_address()
    reward_address = create_nimiq_address()

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
min_peers = 1

[consensus]
network = "dev-albatross"
min_peers = 1

[database]
path = "{path}"

[log]
level = "debug"
timestamps = true

[validator]
validator_address = "{validator_address}"
validator_key_file = "{path}/validator_key.dat"
validator_key = "{validator_key}"
fee_key_file = "{path}/fee_key.dat"
fee_key = "{fee_key}"
warm_key_file = "{path}/warm_key.dat"
warm_key = "{warm_address}"
    """.format(
            port=str(9101 + i),
            path="temp-state/dev/{}".format(i+1),  # str(path),
            validator_address=validator_address["address"],
            validator_key=validator_key["private_key"],
            fee_key=reward_address["private_key"],
            warm_address=warm_address["private_key"]
        ))

    return {
        "validator_key": validator_key,
        "validator_address": validator_address,
        "warm_address": warm_address,
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
        validator["validator_key"]["public_key"][0:16]))

# Create seed node configuration
create_seed(output / "seed", 1)
print("Created seed node configuration")

# Create spammer node configuration
create_spammer(output / "spammer", 1)
print("Created spammer configuration")

# Genesis configuration
print("Writing genesis config")
with (output / "dev-albatross.toml").open("wt") as f:
    f.write("""
name = "dev-albatross"
seed_message = "Albatross DevNet"
signing_key = "9c4a1b36bf0a0a97b9d96713ee8cce211d321cdb368e06e53355c34a9733463f5b1897200f6af83fd3ea54f15bcbd9a5da772c79b0b06bd41fb95e06ebc4ba4174abe405c8339ac7540789b553d9454646649705cc5097e6643cbb0c0fb50000"
timestamp="{timestamp}"
    """.format(
        # timestamp=datetime.utcnow().isoformat()
        timestamp="2021-07-15T00:00:00.000+00:00"
    ))
    for validator in validators:
        f.write("""
[[validators]]
validator_address = "{validator_address}"
warm_address      = "{warm_address}"
reward_address    = "{reward_address}"
validator_key     = "{validator_key}"
        """.format(
            validator_address=validator["validator_address"]["address"],
            warm_address=validator["warm_address"]["address"],
            reward_address=validator["reward_address"]["address"],
            validator_key=validator["validator_key"]["public_key"]
        ))
    f.write("""
[[accounts]]
address = "NQ55 X103 EJXG 3S2U 3JNE L779 560T 3LNE 7S60"
# private_key = "a24591648e20642fe5107d0285c1cc35d67e2033a92566f1217fbd3a14e07abc"
balance = 10_000_000_00000

[[accounts]]
address = "NQ87 HKRC JYGR PJN5 KQYQ 5TM1 26XX 7TNG YT27"
# private_key = "3336f25f5b4272a280c8eb8c1288b39bd064dfb32ebc799459f707a0e88c4e5f"
balance = 10_000_000_00000

[[accounts]]
address = "NQ24 M1E1 YYY1 C280 GAEQ ULL4 U31R QH04 HCM7"
# private_key = "6ca225de8c2a091a31ae48645453641069ae8a9d3158e9d6e004b417661af500"
balance = 10_000_000_00000

[[accounts]]
address = "NQ08 6R71 Y0GB GSXM YQFH X3P3 A53E 3G6K RBVR"
# private_key = "5899a573451f72a4a1e58c7de3e091a1846d14bd82c98e4bfdaf1857986de7d8"
balance = 10_000_000_00000

[[accounts]]
address = "NQ62 GH6J A1CK MKQT Q4A1 2NCB KLVF R4K9 2BA1"
# private_key = "652be07036bf791644260eaa388534d7dbecb579c69bf3b70c0714ae7d5fdcc2"
balance = 10_000_000_00000

[[accounts]]
address = "NQ98 H463 YTJQ M717 MQ7L NPVR UJKJ VQ9C RYNF"
# private_key = "e3e552194e1e56fb47ccc6eb8becea1c1b813ec23ae7613edff12be152a2e812"
balance = 10_000_000_00000

[[accounts]]
address = "NQ41 EBPT ME3M TMUP SMCB 4V44 QQTL N7Q0 MU9R"
# private_key = "c88cb69af940cc58a1f5aa8f1d943b53893a913af48d873c2e83169644b30edc"
balance = 10_000_000_00000

[[accounts]]
address = "NQ40 GCAA U3UX 8BKD GUN0 PG3T 17HA 4X5H TXVE"
# private_key = "1ef7aad365c195462ed04c275d47189d5362bbfe36b5e93ce7ba2f3add5f439b"
balance = 10_000_000_00000

    """)

print("Writing configuration")
with (output / "validators.json").open("wt") as f:
    json.dump(validators, f)
