import logging
import random
import signal
import sys
import time
import tomli
from jinja2 import Environment, FileSystemLoader
from topology_settings import TopologySettings
from control_settings import ControlSettings
from validator import Validator
from node import Seed, RegularNode
from spammer import Spammer


class Topology:
    """
    General topology

    :param topology_settings: General topology settings.
    :type topology_settings: TopologySettings
    """

    def __init__(self, topology_settings: TopologySettings):
        self.topology_settings = topology_settings
        # List of seed nodes
        self.seed_nodes = []
        # All non restartable nodes
        self.endless_nodes = []
        # All restartable nodes
        self.restartable_nodes = []
        self.latest_block_number = 0
        nimiq_dir = topology_settings.get_nimiq_dir()
        self.jinja_env = Environment(
            loader=FileSystemLoader(f"{nimiq_dir}/scripts/devnet/templates/"),
            trim_blocks=True, lstrip_blocks=True)
        state_dir = topology_settings.get_state_dir()
        self.result_file = f"{state_dir}/RESULT.TXT"

    def __generate_genesis(self, validators: list, spammers: list):
        """
        Generates the genesis TOML file
        """
        # First we need to transform a little the data (from list of objects
        # to list of dicts)
        validators_data = []
        spammers_data = []
        for validator in validators:
            data = dict()
            data['validator_address'] = validator.get_address_keypair()[
                'address']
            data['signing_key'] = validator.get_signing_keypair()[
                'public_key']
            data['voting_key'] = validator.get_voting_keypair()['public_key']
            data['reward_address'] = validator.get_reward_address_keypair()[
                'address']
            validators_data.append(data)
        for spammer in spammers:
            data = dict()
            data['address'] = spammer.get_address()
            spammers_data.append(data)

        # Now read and render the template
        template = self.jinja_env.get_template("dev-albatross-genesis.toml.j2")
        content = template.render(
            validators=validators_data, spammers=spammers_data)
        conf_dir = self.topology_settings.get_conf_dir()
        filename = f"{conf_dir}/dev-albatross.toml"
        with open(filename, mode="w", encoding="utf-8") as message:
            message.write(content)
            logging.info(f"Generated genesis file in {filename}")

    def __parse_toml_topology(self, topology: dict):
        """
        Parses a dictionary containing a topology described by a TOML

        :return: The list of validators, seeds, spammers and regular nodes
        :rtype: tuple of (list, list, list, list)
        """
        validators = []
        seeds = []
        spammers = []
        regular_nodes = []
        port_bases = {'validator': {'listen': 9200, 'metrics': 9600},
                      'seed': {'listen': 9100, 'metrics': 9500},
                      'spammer': {'listen': 9900, 'metrics': 9950},
                      'node': {'listen': 9300, 'metrics': 9700}}
        containerized = self.topology_settings.is_env_containerized()
        if not all(key in ['validator', 'seed', 'spammer', 'node']
                   for key in topology):
            raise Exception("Invalid TOML configuration file")
        for key in topology:
            for count, node in enumerate(topology[key]):
                if 'sync_mode' not in node:
                    raise Exception("Missing sync_mode for a node")
                elif node['sync_mode'] not in ["full", "history", "light"]:
                    raise Exception(
                        "Unexpected sync_mode for a node: "
                        f"{node['sync_mode']}")
                if 'restartable' not in node:
                    raise Exception("Missing restartable for a node")
                elif not isinstance(node['restartable'], bool):
                    raise Exception(
                        "Unexpected restartable value for a node: "
                        f"{node['restartable']}")
                name = f"{key}{count+1}"
                if 'enable_metrics' in node:
                    if containerized:
                        metrics_port = 9100
                    else:
                        metrics_port = port_bases[node]['metrics'] + count
                    metrics = {'port': metrics_port}
                else:
                    metrics = None
                # Create objects depending on type:
                if containerized:
                    port = 8443
                else:
                    port = port_bases[key]['listen'] + count
                if key == 'validator':
                    topology_node = Validator(
                        name, port, self.topology_settings, node['sync_mode'],
                        metrics=metrics)
                    validators.append(topology_node)
                elif key == 'seed':
                    topology_node = Seed(
                        name, port, self.topology_settings, node['sync_mode'],
                        metrics=metrics)
                    seeds.append(topology_node)
                    self.seed_nodes.append(topology_node)
                elif key == 'node':
                    topology_node = RegularNode(
                        name, port, self.topology_settings, node['sync_mode'],
                        metrics=metrics)
                    regular_nodes.append(topology_node)
                elif key == 'spammer':
                    if 'tpb' not in node:
                        raise Exception("Missing tpb for a spammer")
                    elif not isinstance(node['tpb'], int):
                        raise Exception(
                            "Unexpected tpb value for a spammer: "
                            f"{node['tpb']}")
                    topology_node = Spammer(
                        name, port, self.topology_settings, node['tpb'],
                        node['sync_mode'], metrics=metrics)
                    spammers.append(topology_node)
                # Now add them to the nodes attributes
                if node['restartable']:
                    self.restartable_nodes.append(topology_node)
                else:
                    self.endless_nodes.append(topology_node)

        return (validators, seeds, spammers, regular_nodes)

    def load(self, topology_def: str):
        """
        Loads and builds a topology based on a TOML description

        :return: The path of the directory of the log files
        :rtype: str
        """
        with open(topology_def, 'rb') as fileObj:
            description = tomli.load(fileObj)
            (validators, seeds, spammers,
             regular_nodes) = self.__parse_toml_topology(description)
            self.__generate_genesis(validators, spammers)
            non_seed_nodes = validators + spammers + regular_nodes
            seed_addresses = []
            for seed in seeds:
                if self.topology_settings.is_env_containerized():
                    seed_addresses.append(f"/dns4/{seed.get_name()}/tcp/"
                                          f"{seed.get_listen_port()}/ws")
                else:
                    seed_addresses.append("/ip4/127.0.0.1/tcp/"
                                          f"{seed.get_listen_port()}/ws")
                seed.generate_config_files(self.jinja_env)
            for node in non_seed_nodes:
                node.generate_config_files(self.jinja_env, seed_addresses)

    def __run_monitor_for(self, mon_time: int, exp_down_nodes: list = []):
        """
        Runs and monitor all nodes for a period of time

        :param time: Time in seconds for which all nodes will keep running and
            will be monitored
        :param time: int
        """
        all_nodes = self.endless_nodes + self.restartable_nodes
        panicked_nodes = []
        terminated_nodes = []
        for _ in range(mon_time):
            for node in all_nodes:
                if node.check_panics():
                    panicked_nodes.append(node)
                elif node not in exp_down_nodes and node.poll() is not None:
                    terminated_nodes.append(node)
            if len(panicked_nodes) != 0:
                warnings = self.__cleanup()
                with open(self.result_file, 'a+') as result_file:
                    result_file.write("PANIC\n")
                raise Exception(
                    "Panic found in nodes: {}".format(
                        list(map(lambda node: node.get_name(),
                                 panicked_nodes))))
            if len(terminated_nodes) != 0:
                warnings = self.__cleanup()
                with open(self.result_file, 'a+') as result_file:
                    result_file.write("TERMINATED\n")
                raise Exception(
                    "Process for nodes '{}' has unexpectedly "
                    "terminated".format(
                        list(map(lambda node: node.get_name(),
                                 terminated_nodes))))
            time.sleep(1)

        # Now check if nodes were able to produce blocks in mon_time
        latest_block_number = 0
        for node in all_nodes:
            node_latest_block_number = node.get_latest_block_number()
            if node_latest_block_number > latest_block_number:
                latest_block_number = node_latest_block_number
        if latest_block_number <= self.latest_block_number:
            warnings = self.__cleanup()
            with open(self.result_file, 'a+') as result_file:
                result_file.write("CHAIN-STALL\n")
            raise Exception(
                f"No new blocks were produced after {mon_time}s")
        else:
            logging.info(f"Latest block number: {latest_block_number}")

    def __cleanup(self):
        """
        Ends all remaining processes

        :return: A dictionary containing possible warnings for all nodes
        :rtype: dict
        """
        all_nodes = self.endless_nodes + self.restartable_nodes
        warnings = dict()
        logging.info("Killing all nodes")
        for node in all_nodes:
            node_warnings = list()
            if node.check_slow_lock_acquisition():
                with open(self.result_file, 'a+') as result_file:
                    result_file.write("SLOW_LOCK_ACQUISITION\n")
                node_warnings.append('slow_lock')
            if node.check_long_lock_hold():
                with open(self.result_file, 'a+') as result_file:
                    result_file.write("LONG_LOCK_HOLD_TIME\n")
                node_warnings.append('long_lock')
            if node.check_deadlocks():
                with open(self.result_file, 'a+') as result_file:
                    result_file.write("DEADLOCK\n")
                node_warnings.append('deadlock')
            if len(node_warnings) > 0:
                warnings[node.get_name()] = node_warnings
            node.kill(False)
        return warnings

    def __sigint_handler(self, _signum, _frame):
        """
        Handler for SINGINT (CTRL-c). This is essentially a wrapper for
        `__cleanup`
        """
        logging.info("Received CTRL-C. Cleaning up")
        self.__cleanup()
        sys.exit(0)

    def run(self, control_settings: ControlSettings):
        """
        Main control function that runs a topology
        """
        all_nodes = self.endless_nodes + self.restartable_nodes

        # Register a handler for SIGINT (CTRL-c)
        signal.signal(signal.SIGINT, self.__sigint_handler)

        # Continue with the rest of the nodes
        for node in all_nodes:
            logging.info(f"Building {node.get_name()}")
            node.build()

        # Start all nodes, starting first with seeds
        for seed in self.seed_nodes:
            logging.info(f"Starting {seed.get_name()}")
            seed.run()
            # Wait some seconds for seed to be up
            time.sleep(3)

        # Continue with the rest of the nodes
        for node in all_nodes:
            # Seed nodes are skipped since they are started first
            if node in self.seed_nodes:
                continue
            logging.info(f"Starting {node.get_name()}")
            node.run()
            # Wait a second for node to be up
            time.sleep(1)

        # Let the validators produce blocks for 30 seconds
        time.sleep(30)

        if control_settings.is_continuous():
            # Continuous mode
            monitor_interval = control_settings.get_monitor_interval()
            while True:
                logging.info(
                    f"Producing blocks for {monitor_interval}s with all nodes "
                    "up")
                self.__run_monitor_for(monitor_interval)

        else:
            restart_settings = control_settings.get_restart_settings()
            max_restarts = restart_settings.get_max_restarts()
            sim_kills = restart_settings.get_sim_kills()
            up_time = restart_settings.get_up_time()
            down_time = restart_settings.get_down_time()
            erase_state = restart_settings.get_erase_state()
            erase_db = restart_settings.get_erase_db()
            for _ in range(max_restarts):
                # Select nodes to kill
                nodes_to_restart = random.sample(
                    self.restartable_nodes, k=sim_kills)
                for node in nodes_to_restart:
                    logging.info(f"Killing {node.get_name()}")
                    node.kill(True)
                    if erase_state:
                        logging.info(
                            f"Erasing state for {node.get_name()}")
                        node.erase_state()
                    elif erase_db:
                        logging.info(
                            f"Erasing DB for {node.get_name()}")
                        node.erase_db()

                logging.info(
                    f"Running with node(s) down for {down_time}s")
                self.__run_monitor_for(
                    down_time, exp_down_nodes=nodes_to_restart)

                for node in nodes_to_restart:
                    logging.info(
                        f"Restarting {node.get_name()}")
                    node.run()
                    time.sleep(2)

                logging.info(
                    f"Producing blocks for {up_time}s with all nodes up")
                self.__run_monitor_for(up_time)

            time.sleep(30)
            warnings = self.__cleanup()
