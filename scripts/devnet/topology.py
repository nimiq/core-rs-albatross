import logging
import random
import time
import signal
import tomli
import sys
import subprocess
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
        self.jinja_env = Environment(
            loader=FileSystemLoader(topology_settings.nimiq_dir +
                                    "/scripts/devnet/templates/"),
            trim_blocks=True, lstrip_blocks=True)
        self.result_file = topology_settings.get_state_dir() + "/" + \
            "RESULT.TXT"

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
        filename = self.topology_settings.get_conf_dir() + \
            '/' + "dev-albatross.toml"
        with open(filename, mode="w", encoding="utf-8") as message:
            message.write(content)
            logging.info("Generated genesis file in {}".format(filename))

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
        if not all(key in ['validator', 'seed', 'spammer', 'node']
                   for key in topology):
            raise Exception("Invalid TOML configuration file")
        for key in topology:
            for count, node in enumerate(topology[key]):
                if 'sync_mode' not in node:
                    raise Exception("Missing sync_mode for a node")
                elif node['sync_mode'] not in ["full", "history", "light"]:
                    raise Exception(
                        "Unexpected sync_mode for a node: {}".format(
                            node['sync_mode']))
                if 'restartable' not in node:
                    raise Exception("Missing restartable for a node")
                elif not isinstance(node['restartable'], bool):
                    raise Exception(
                        "Unexpected restartable value for a node: {}".format(
                            node['restartable']))
                name = "{}{}".format(key, count+1)
                if 'enable_metrics' in node:
                    metrics = {'port': port_bases[node]['metrics'] + count}
                else:
                    metrics = None
                # Create objects depending on type:
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
                            "Unexpected tpb value for a spammer: {}".format(
                                node['tpb']))
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
            seed_ports = []
            for seed in seeds:
                seed_ports.append(seed.get_listen_port())
                seed.generate_config_files(self.jinja_env)
            for node in non_seed_nodes:
                node.generate_config_files(self.jinja_env, seed_ports)

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
                subprocess.run(["echo", "PANIC", ">>", self.result_file],
                               check=True, capture_output=True)
                raise Exception(
                    "Panic found in nodes: {}".format(
                        list(map(lambda node: node.get_name(),
                                 panicked_nodes))))
            if len(terminated_nodes) != 0:
                warnings = self.__cleanup()
                subprocess.run(["echo", "TERMINATED", ">>", self.result_file],
                               check=True, capture_output=True)
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
            subprocess.run(["echo", "CHAIN-STALL", ">>", self.result_file],
                           check=True, capture_output=True)
            raise Exception(
                "No new blocks were produced after {}s".format(mon_time))
        else:
            logging.info("Latest block number: {}".format(latest_block_number))

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
                subprocess.run(["echo", "SLOW_LOCK_ACQUISITION", ">>",
                                self.result_file], check=True,
                               capture_output=True)
                node_warnings.append('slow_lock')
            if node.check_long_lock_hold():
                subprocess.run(["echo", "LONG_LOCK_HOLD_TIME", ">>",
                                self.result_file], check=True,
                               capture_output=True)
                node_warnings.append('long_lock')
            if node.check_deadlocks():
                subprocess.run(["echo", "DEADLOCK", ">>", self.result_file],
                               check=True, capture_output=True)
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

        # Start all nodes, starting first with seeds
        for seed in self.seed_nodes:
            logging.info("Starting {}".format(seed.get_name()))
            seed.run()
            # Wait some seconds for seed to be up
            time.sleep(3)

        # Continue with the rest of the nodes
        for node in all_nodes:
            # Seed nodes are skipped since they are started first
            if node in self.seed_nodes:
                continue
            logging.info("Starting {}".format(node.get_name()))
            node.run()
            # Wait a second for node to be up
            time.sleep(1)

        # Let the validators produce blocks for 30 seconds
        time.sleep(30)

        if control_settings.is_continuous():
            # Continuous mode
            while True:
                logging.info(
                    "Producing blocks for {}s with all nodes up".format(
                        up_time))
                self.__run_monitor_for(up_time)

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
                    logging.info("Killing {}".format(node.get_name()))
                    node.kill(True)
                    time.sleep(1)
                    if erase_state:
                        logging.info(
                            "Erasing state for {}".format(node.get_name()))
                        node.erase_state()
                    elif erase_db:
                        logging.info(
                            "Erasing DB for {}".format(node.get_name()))
                        node.erase_db()

                logging.info(
                    "Running with node(s) down for {}s".format(down_time))
                self.__run_monitor_for(
                    down_time, exp_down_nodes=nodes_to_restart)

                for node in nodes_to_restart:
                    logging.info("Restarting {}".format(node.get_name()))
                    node.run()
                    time.sleep(2)

                logging.info(
                    "Producing blocks for {}s with all ""nodes up".format(
                        up_time))
                self.__run_monitor_for(up_time)

            time.sleep(30)
            warnings = self.__cleanup()
