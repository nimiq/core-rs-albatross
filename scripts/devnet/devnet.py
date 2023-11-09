import argparse
import subprocess
import logging
import os
import uuid
from datetime import datetime
from pathlib import Path
from topology_settings import LokiSettings, TopologySettings, Environment
from control_settings import ControlSettings, RestartSettings
from topology import Topology

LOG_LEVELS = ["DEBUG", "INFO", "WARNING", "ERROR", "CRITICAL"]
DEFAULT_LOG_LEVEL = "INFO"


def setup_logging(args):
    """
    Sets-up logging according to the arguments received.

    :params args: Command line arguments of the program
    :type args: Namespace
    """
    # Adjust log level accordingly
    log_level = LOG_LEVELS.index(DEFAULT_LOG_LEVEL)
    for adjustment in args.log_level or ():
        log_level = min(len(LOG_LEVELS) - 1, max(log_level + adjustment, 0))

    log_level_name = LOG_LEVELS[log_level]
    logging.getLogger().setLevel(log_level_name)
    logging.basicConfig(
        format='%(asctime)s %(levelname)-8s %(message)s',
        level=logging.INFO,
        datefmt='%Y-%m-%d %H:%M:%S')


def check_positive(value):
    int_value = int(value)
    if int_value <= 0:
        raise argparse.ArgumentTypeError(
            f"{value}: Expected positive (non zero) integer value")
    return int_value


def parse_args():
    parser = argparse.ArgumentParser()
    parser.add_argument('-n', '--namespace', type=str, default="default",
                        help="Namespace to be generated and assigned to all "
                        "the generated resources. k8s environment only.")
    parser.add_argument('-t', '--topology', type=str, required=True,
                        help="Input TOML topology file")
    parser.add_argument('-o', "--output", metavar='DIR', type=str,
                        help="Output directory", default="/tmp/nimiq-devnet")
    parser.add_argument('-R', '--release', action='store_true', help="Compiles"
                        " and runs the code in release mode")
    parser.add_argument('-m', "--metrics", action="store_true",
                        help="Adds configuration to enable metrics")
    parser.add_argument("--run-environment", default=None,
                        help="sent to Loki, like \"ci\", \"devnet\", default: "
                        "\"unknown\"")
    parser.add_argument('-e', "--erase", action='store_true',
                        help="Erase all of the validator state as part of "
                        "restarting it")
    parser.add_argument('-r', "--restarts", type=check_positive, default=10,
                        help="The number of times you want to kill/restart "
                        "validators (by default 10 times)(0 means no restarts)"
                        )
    parser.add_argument('-c', "--continuous", action='store_true',
                        help="In continuous mode the script runs until it is "
                        "killed (or it finds an error)")
    parser.add_argument('-k', "--kills", type=int, default=1,
                        help="How many validators are killed each cycle, by "
                        "default just 1")
    parser.add_argument('-dt', "--down-time", type=check_positive, default=10,
                        help="Time in seconds that validators are taken down, "
                        "by default 10s")
    parser.add_argument('-ut', "--up-time", type=check_positive, default=40,
                        help="Time in seconds during which all validators are "
                        "up, by default 40s")
    parser.add_argument('-d', "--db", action='store_true',
                        help="Erases only the database state of the validator "
                        "as part of restarting it")
    parser.add_argument("--verbose", "-v", dest="log_level",
                        action="append_const", const=-1)
    parser.add_argument("--dry", action='store_true', help="Only generate "
                        "configuration files. This won't run a devnet and will"
                        " dismiss any configuration option except for the "
                        "topology")
    parser.add_argument("--env", type=Environment.from_string,
                        choices=list(Environment), default=Environment.LOCAL,
                        help="Environment in which the devnet will be run: "
                        "local, docker-compose or k8s")
    return parser.parse_args()


def main():

    args = parse_args()
    setup_logging(args)

    if args.continuous:
        control_settings = ControlSettings(monitor_interval=args.up_time)
    else:
        restart_settings = RestartSettings(
            args.up_time, args.down_time, args.restarts, args.kills, args.db,
            args.erase)
        control_settings = ControlSettings(restart_settings)
    now = datetime.now()
    ts = now.strftime("%Y%m%d_%H%M%S")

    # Parse the git top level directory
    nimiq_dir = subprocess.check_output(
        ['git', 'rev-parse', '--show-toplevel'], text=True).rstrip()

    # Create the logs dir
    logs_dir = f"{nimiq_dir}/temp-logs/{ts}"
    Path(logs_dir).mkdir(parents=True, exist_ok=False)

    # Create the conf dir
    conf_dir = f"{logs_dir}/conf"
    Path(conf_dir).mkdir(parents=False, exist_ok=False)

    # Create the state dir
    if args.env != Environment.LOCAL:
        state_dir = f"/home/nimiq"
    else:
        state_dir = f"{nimiq_dir}/temp-state/{ts}"
        Path(state_dir).mkdir(parents=True, exist_ok=False)

    # Create the loki settings
    loki_settings = None
    loki_url = os.getenv("NIMIQ_LOKI_URL")
    if args.run_environment is not None and loki_url is not None:
        run_id = str(uuid.uuid4())
        loki_labels_env = os.getenv("NIMIQ_LOKI_LABELS")
        loki_extra_fields_env = os.getenv("NIMIQ_LOKI_EXTRA_FIELDS")
        loki_labels = {"environment": args.run_environment}
        loki_extra_fields = {"nimiq_run_id": run_id}
        if loki_labels_env:
            loki_labels.update(dict(x.split("=", 1)
                               for x in loki_labels_env.split(":")))
        if loki_extra_fields_env:
            loki_extra_fields.update(dict(x.split("=", 1)
                                     for x in loki_extra_fields_env.split(":"))
                                     )
        loki_settings = LokiSettings(loki_url, loki_labels, loki_extra_fields)

    topology_settings = TopologySettings(
        nimiq_dir, logs_dir, conf_dir, state_dir, args.release,
        args.namespace, args.env, loki_settings=loki_settings)

    # Now create topology object and run it
    topology = Topology(topology_settings)
    topology.load(args.topology)
    # For now we do not support control of docker-compose and k8s environments
    if not args.dry and not args.env != Environment.LOCAL:
        topology.run(control_settings)
    else:
        logging.warning("Only generating configuration files. Topology control"
                        " for this environment is currently not supported")


if __name__ == "__main__":
    main()
