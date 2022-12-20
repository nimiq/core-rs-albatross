import subprocess
from topology_settings import TopologySettings


def __nimiq_schnorr(topology_settings: TopologySettings):
    command = ["cargo", "run"]
    if topology_settings.get_release():
        command.append("--release")
    command.extend(["--bin", "nimiq-address"])
    output = subprocess.check_output(
        command, text=True, cwd=topology_settings.get_nimiq_dir(),
        stderr=subprocess.DEVNULL)
    return output.splitlines()


def __nimiq_bls(topology_settings: TopologySettings):
    command = ["cargo", "run"]
    if topology_settings.get_release():
        command.append("--release")
    command.extend(["--bin", "nimiq-bls"])
    output = subprocess.check_output(
        command, text=True, cwd=topology_settings.get_nimiq_dir(),
        stderr=subprocess.DEVNULL)
    return output.splitlines()


def create_bls_keypair(topology_settings: TopologySettings):
    lines = []
    for line in __nimiq_bls(topology_settings):
        line = line.strip()
        if line and not line.startswith("#"):
            lines.append(line)
    return {
        "public_key": lines[0],
        "private_key": lines[1]
    }


def create_schnorr_keypair(topology_settings: TopologySettings):
    lines = []
    for i, l in enumerate(__nimiq_schnorr(topology_settings)):
        lines.append(l.split(":")[1].strip())
    return {
        "address": lines[0],
        "address_raw": lines[1],
        "public_key": lines[2],
        "private_key": lines[3]
    }
