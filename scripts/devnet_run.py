import sh
import json
from pathlib import Path
from sys import argv


TARGET = Path("/home/janosch/nimiq/dev/core-rs-albatross/target/debug")
nimiq_client = sh.Command(str(TARGET / "nimiq-client"))


def run_client(config_path, i):
    def process_out(line):
        print("[validator{:02}/stdout] {}".format(i, line), end="")

    def process_err(line):
        print("[validator{:02}/stderr] {}".format(i, line), end="")

    return nimiq_client(config=str(config_path), _bg=True, _out=process_out, _err=process_err)


try:
    path = Path(argv[1])
except IndexError:
    print("Usage: {} PATH".format(argv[0]))
    exit(1)


with (path / "validators.json").open("rt") as f:
    validators = json.load(f)


print("Running validators")
processes = []
for i, validator in enumerate(validators):
    process = run_client(Path(validator["path"]) / "client.toml", i)
    print("Validator {} started with PID={}".format(i, process.pid))
    processes.append(process)

try:
    print("Waiting for processes to terminate")
    for i, process in enumerate(processes):
        process.wait()
        print("Validator {} (PID={}) terminated".format(i, process.pid))
except KeyboardInterrupt:
    print("Keyboard interrupt. Terminating processes.")
    for process in processes:
        process.terminate()

