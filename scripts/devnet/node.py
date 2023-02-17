import subprocess
import signal
import os
import re
from enum import Enum
from typing import Optional
from jinja2 import Environment
from pathlib import Path

from topology_settings import TopologySettings


class NodeType(Enum):
    """
    Type of a node within a Devnet network
    """
    REGULAR_NODE = 0
    """Regular node"""
    SEED = 1
    """Seed node"""
    VALIDATOR = 2
    """Validator node"""
    SPAMMER = 3
    """Spammer mode"""


class Node:
    """
    Devnet node

    :param type: Type of the node.
    :type type: NodeType
    :param name: Name of the node.
    :type name: str
    :param nimiq_exec: Name of the Nimiq executable.
    :type nimiq_exec: str
    :param listen_port: Port this node will be listening to connections.
    :type listen_port: int
    :param topology_settings: General topology settings
    :type topology_settings: TopologySettings
    :param sync_mode: The node sync mode (history, full or light)
    :type sync_mode: str
    :param metrics: Optional metrics settings
    :type metrics: Optional[dict]
    """

    def __init__(self, type: NodeType, name: str, nimiq_exec: str,
                 listen_port: int, topology_settings: TopologySettings,
                 sync_mode: str = "full", metrics: Optional[dict] = None,
                 nimiq_exec_extra_args: list = []):
        self.type = type
        self.name = name
        self.nimiq_exec = nimiq_exec
        self.listen_port = listen_port
        self.metrics = metrics
        self.topology_settings = topology_settings
        self.nimiq_exec_extra_args = nimiq_exec_extra_args
        self.conf_dir = topology_settings.get_conf_dir() + "/" + self.name
        Path(self.conf_dir).mkdir(parents=False, exist_ok=False)
        self.state_dir = topology_settings.get_state_dir() + "/" + self.name
        Path(self.state_dir).mkdir(parents=False, exist_ok=False)
        self.sync_mode = sync_mode
        self.process = None

    def get_type(self):
        """
        Gets the type of node of the current node

        :return: The type of node.
        :rtype: NodeType
        """
        return self.type

    def get_name(self):
        """
        Gets the name assigned to the current node

        :return: The name of the node.
        :rtype: str
        """
        return self.name

    def get_listen_port(self):
        """
        Gets the port number this node is listening to

        :return: The listen port of the node.
        :rtype: int
        """
        return self.listen_port

    def get_metrics(self):
        """
        Gets the metrics settings of the current node

        :return: The metrics settings of the node.
        :rtype: Optional[dict]
        """
        return self.metrics

    def get_sync_mode(self):
        """
        Gets the sync mode of the current node

        :return: The sync mode of the node.
        :rtype: str
        """
        return self.sync_mode

    def get_log(self):
        """
        Gets the log file of the current node

        :return: The log file of the node.
        :rtype: str
        """
        return self.topology_settings.get_logs_dir() + "/" + self.name + ".log"

    def get_conf_toml(self):
        """
        Gets the TOML configuration file of the current node

        :return: The TOML configuration file of the node.
        :rtype: str
        """
        return self.conf_dir + "/" + self.name + ".toml"

    def get_conf_yaml(self):
        """
        Gets the YAML configuration file of the current node

        :return: The YAML configuration file of the node.
        :rtype: str
        """
        return self.conf_dir + "/" + self.name + ".yml"

    def get_state_dir(self):
        """
        Gets the state directory of the current node

        :return: The state directory of yhe node.
        :rtype: str
        """
        return self.state_dir

    def get_db(self):
        """
        Gets the database path of the current node

        :return: The DB path of the current node.
        :rtype: str
        """
        return self.get_state_dir() + "/" + "devalbatross-history-consensus"

    def run(self):
        """
        Runs the process for the current node
        """
        if self.process is not None:
            raise Exception(
                f"Process for {self.name} has already been started")
        # Prepare the genesis environment variable
        genesis_file = self.topology_settings.get_conf_dir() + \
            '/' + "dev-albatross.toml"
        env = {
            **os.environ,
            'NIMIQ_OVERRIDE_DEVNET_CONFIG': genesis_file,
        }
        # Open the log file
        log_file = open(self.get_log(), 'a+')
        # Prepare the client execution command
        command = ["cargo", "run", "--bin", self.nimiq_exec]
        if self.topology_settings.get_release():
            command.append("--release")
        command.extend(["--", "-c",
                        self.get_conf_toml()])
        command.extend(self.nimiq_exec_extra_args)
        self.process = subprocess.Popen(
            command, env=env, cwd=self.topology_settings.get_nimiq_dir(),
            stdout=log_file, stderr=log_file)

    def kill(self, will_be_restarted: bool):
        """
        Kills the process for the current node

        :param will_be_restarted: Flag indicating if the kill is for a later
            restart
        :type will_be_restarted: bool
        """
        if self.process is not None:
            self.process.send_signal(signal.SIGINT)
            if will_be_restarted:
                log_str = "\n################################## RESTART ######"
                "#####################\n"
                with open(self.get_log(), 'a') as log_file:
                    log_file.write(log_str)
                self.process = None

    def check_panics(self):
        """
        Checks the log to see if a panic happened

        :return: A boolean indicating if a panic has happened or not
        :rtype: bool
        """
        panics = subprocess.run(["grep", "ERROR.*panic[[:blank:]]",
                                 self.get_log()], capture_output=True,
                                shell=False, text=True).stdout
        return len(panics) != 0

    def check_long_lock_hold(self):
        """
        Checks the log to see if a long lock hold has happened

        :return: A boolean indicating if a long lock hold has happened
        :rtype: bool
        """
        long_lock = subprocess.run(["grep", "lock.held.for.a.long.time",
                                    self.get_log()], capture_output=True,
                                   text=True).stdout
        return len(long_lock) != 0

    def check_slow_lock_acquisition(self):
        """
        Checks the log to see if a slow lock acquisition has happened

        :return: A boolean indicating if a slow lock acquisition has happened
        :rtype: bool
        """
        slow_lock = subprocess.run(["grep", "slow.*took", self.get_log(
        )], capture_output=True, text=True).stdout
        return len(slow_lock) != 0

    def check_deadlocks(self):
        """
        Checks the log to see if a deadlock has happened

        :return: A boolean indicating if a deadlock has happened
        :rtype: bool
        """
        deadlock = subprocess.run(["grep", "deadlock", self.get_log(
        )], capture_output=True, text=True).stdout
        return len(deadlock) != 0

    def poll(self):
        """
        Polls the process for the current node.

        :return: The return code of the process if it ended, None if it hasn't
            finished
        :rtype: int ot None
        """
        return self.process.poll()

    def erase_db(self):
        """
        Erases DB of the current node
        """
        # Open the log file
        log_file = open(self.get_log(), 'a+')
        subprocess.run(["rm", "-r", self.get_db()],
                       check=True, capture_output=True)
        log_str = "\n################################## NODE DB DELETED #####"
        "######################\n"
        log_file.write(log_str)
        log_file.close()

    def erase_state(self):
        """
        Erases the whole state of the current node
        """
        # Open the log file
        log_file = open(self.get_log(), 'a+')
        # We need to use shell to use the '*' wildcard such that the state
        # dir doesn't get removed as well.
        # Also the command needs to be a string
        subprocess.run(f"rm -r {self.get_state_dir()}/*",
                       shell=True, check=True, capture_output=True)
        log_str = "\n################################## NODE STATE DELETED ##"
        "#########################\n"
        log_file.write(log_str)
        log_file.close()

    def get_latest_block_number(self):
        """
        Gets the latest block number of the current node
        """
        last_accepted_block_line = subprocess.Popen(
            ['tail', '-1'], stdin=subprocess.PIPE)
        grep_output = subprocess.run(["grep", "-A0", "-B0", "Accepted.block.*",
                                      self.get_log()],
                                     capture_output=True, text=True)
        last_accepted_block_line = subprocess.run(['tail', '-1'],
                                                  input=grep_output.stdout,
                                                  capture_output=True,
                                                  text=True).stdout
        # Remove ANSI escape sequences
        ansi_escape = re.compile(r'\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])')
        last_accepted_block_line = ansi_escape.sub(
            '', last_accepted_block_line)
        regex = re.compile(r'Accepted block block=#(\d*)')
        parsed_block = regex.search(last_accepted_block_line)
        if parsed_block is None:
            return 0
        else:
            return int(parsed_block.group(1))


class RegularNode(Node):
    """
    Devnet regular node

    :param name: Name of the node.
    :type name: str
    :param listen_port: Port this node will be listening to connections.
    :type listen_port: int
    :param topology_settings: General topology settings
    :type topology_settings: TopologySettings
    :param sync_mode: The node sync mode (history, full or light)
    :type sync_mode: str
    :param metrics: Optional metrics settings
    :type metrics: Optional[dict]
    """

    def __init__(self, name: str, listen_port: int,
                 topology_settings: TopologySettings, sync_mode: str = "full",
                 metrics: Optional[dict] = None):
        super(RegularNode, self).__init__(NodeType.REGULAR_NODE,
                                          name, "nimiq-client", listen_port,
                                          topology_settings, sync_mode,
                                          metrics)

    def generate_config_files(self, jinja_env: Environment, seed_ports: list):
        """
        Generates configuration file

        :param jinja_env: Jinja2 environment for template rendering
        :type jinja_env: Environment
        :param seed_ports: List of seed ports for the configuration file
        :type seed_ports: List of ints
        """
        # Read and render the TOML template
        template = jinja_env.get_template("node_conf.toml.j2")
        metrics = self.get_metrics()
        loki_settings = self.topology_settings.get_loki_settings()
        if loki_settings is not None:
            loki_settings = loki_settings.format_for_config_file()
            loki_settings['extra_fields']['nimiq_node'] = self.name
        if metrics is not None:
            content = template.render(
                min_peers=3, port=self.get_listen_port(),
                state_path=self.get_state_dir(),
                sync_mode=self.get_sync_mode(), seed_ports=seed_ports,
                metrics=metrics, loki=loki_settings)
        else:
            content = template.render(
                min_peers=3, port=self.get_listen_port(),
                state_path=self.get_state_dir(),
                sync_mode=self.get_sync_mode(), seed_ports=seed_ports,
                loki=loki_settings)
        filename = self.get_conf_toml()
        with open(filename, mode="w", encoding="utf-8") as message:
            message.write(content)


class Seed(Node):
    """
    Devnet seed node

    :param name: Name of the seed node.
    :type name: str
    :param listen_port: Port this node will be listening to connections.
    :type listen_port: int
    :param topology_settings: General topology settings
    :type topology_settings: TopologySettings
    :param sync_mode: The node sync mode (history, full or light)
    :type sync_mode: str
    :param metrics: Optional metrics settings
    :type metrics: Optional[dict]
    """

    def __init__(self, name: str, listen_port: int,
                 topology_settings: TopologySettings, sync_mode: str = "full",
                 metrics: Optional[dict] = None):
        super(Seed, self).__init__(NodeType.SEED,
                                   name, "nimiq-client", listen_port,
                                   topology_settings, sync_mode, metrics)

    def generate_config_files(self, jinja_env: Environment):
        """
        Generates configuration file

        :param jinja_env: Jinja2 environment for template rendering
        :type jinja_env: Environment
        """
        # Read and render the TOML template
        template = jinja_env.get_template("node_conf.toml.j2")
        metrics = self.get_metrics()
        loki_settings = self.topology_settings.get_loki_settings()
        if loki_settings is not None:
            loki_settings = loki_settings.format_for_config_file()
            loki_settings['extra_fields']['nimiq_node'] = self.name
        if metrics is not None:
            content = template.render(
                min_peers=3, port=self.get_listen_port(),
                state_path=self.get_state_dir(),
                sync_mode=self.get_sync_mode(), metrics=metrics,
                loki=loki_settings)
        else:
            content = template.render(
                min_peers=3, port=self.get_listen_port(),
                state_path=self.get_state_dir(),
                sync_mode=self.get_sync_mode(), loki=loki_settings)
        filename = self.get_conf_toml()
        with open(filename, mode="w", encoding="utf-8") as message:
            message.write(content)
