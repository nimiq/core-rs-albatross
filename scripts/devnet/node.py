import os
import subprocess
import signal
import re
import time
from enum import Enum
from typing import Optional
from jinja2 import Environment
from pathlib import Path
from typing import List

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
    :param rpc: Optional rpc settings
    :type rpc: Optional[dict]
    :param metrics: Optional metrics settings
    :type metrics: Optional[dict]
    :param container_image: Optional container image
    :type container_image: Optional[str]
    :param nimiq_exec_extra_args: Optional list of arguments for the nimiq
        client
    :type nimiq_exec_extra_args: List[str]
    """

    def __init__(
            self, type: NodeType, name: str, nimiq_exec: str, listen_port: int,
            topology_settings: TopologySettings, sync_mode: str = "full",
            rpc: Optional[dict] = None, metrics: Optional[dict] = None,
            container_image: Optional[str] = None,
            nimiq_exec_extra_args: List[str] = []):
        self.type = type
        self.name = name
        self.nimiq_exec = nimiq_exec
        self.listen_port = listen_port
        self.rpc = rpc
        self.metrics = metrics
        self.container_image = container_image
        self.topology_settings = topology_settings
        self.nimiq_exec_extra_args = nimiq_exec_extra_args
        self.conf_dir = topology_settings.get_node_conf_dir(self.name)
        Path(self.conf_dir).mkdir(parents=False, exist_ok=False)
        self.state_dir = topology_settings.get_node_state_dir(self.name)
        # Set the container image
        if container_image is None:
            self.container_image = 'ghcr.io/nimiq/core-rs-albatross:latest'
        else:
            self.container_image = container_image
        # Only create a directory for the node state if the node won't be
        # containerized.
        if not topology_settings.is_env_containerized():
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

    def get_rpc(self):
        """
        Gets the rpc settings of the current node

        :return: The rpc settings of the node.
        :rtype: Optional[dict]
        """
        return self.rpc

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
        topology_logs_dir = self.topology_settings.get_logs_dir()
        return f"{topology_logs_dir}/{self.name}.log"

    def get_conf_dir(self):
        """
        Gets the configuration directory of the current node

        :return: The configuration directory of the node.
        :rtype: str
        """
        return self.conf_dir

    def get_conf_toml(self):
        """
        Gets the TOML configuration file of the current node

        :return: The TOML configuration file of the node.
        :rtype: str
        """
        return f"{self.conf_dir}/client.toml"

    def get_conf_yaml(self):
        """
        Gets the YAML configuration file of the current node

        :return: The YAML configuration file of the node.
        :rtype: str
        """
        return f"{self.conf_dir}/{self.name}.yml"

    def get_state_dir(self):
        """
        Gets the state directory of the current node

        :return: The state directory of the node.
        :rtype: str
        """
        return self.state_dir

    def get_db(self):
        """
        Gets the database path of the current node

        :return: The DB path of the current node.
        :rtype: str
        """
        return f"{self.get_state_dir()}/devalbatross-history-consensus"

    def get_container_image(self):
        """
        Gets the container image of the current node

        :return: The container image of the current node.
        :rtype: str
        """
        return self.container_image

    def build(self):
        """
        Builds the code for the current node
        """
        # Prepare the build command.
        # Note: Use the `run` for the client since this will assure that
        # when we call it later, it won't compile the code again.
        # To exit the client, we will call it with `--help`
        command = ["cargo", "run", "--bin", self.nimiq_exec]
        if self.topology_settings.get_release():
            command.append("--release")
        command.extend(["--", "--help"])
        subprocess.run(
            command, cwd=self.topology_settings.get_nimiq_dir(), check=True,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT)

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
            # Try to kill the process with SIGINT
            self.process.send_signal(signal.SIGINT)
            time.sleep(1)
            # If the process is still alive, send SIGKILL
            if self.process is not None:
                self.process.send_signal(signal.SIGKILL)
                time.sleep(1)
            if will_be_restarted:
                log_str = ("\n\n################################## RESTART ###"
                           "########################\n\n")
                with open(self.get_log(), 'a') as log_file:
                    log_file.write(log_str)
                self.process = None

    def check_panics(self):
        """
        Checks the log to see if a panic happened

        :return: A boolean indicating if a panic has happened or not
        :rtype: bool
        """
        panics = subprocess.run(["grep", "ERROR.*panicked[[:blank:]]",
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
        :rtype: int or None
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
        log_str = ("\n\n################################## NODE DB DELETED ###"
                   "########################\n\n")
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
        log_str = ("\n\n################################ NODE STATE DELETED ##"
                   "###########################\n\n")
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
    :param rpc: Optional rpc settings
    :type rpc: Optional[dict]
    :param metrics: Optional metrics settings
    :type metrics: Optional[dict]
    :param container_image: Optional container image
    :type container_image: Optional[str]
    """

    def __init__(self, name: str, listen_port: int,
                 topology_settings: TopologySettings, sync_mode: str = "full",
                 rpc: Optional[dict] = None, metrics: Optional[dict] = None,
                 container_image: Optional[str] = None):
        super(RegularNode, self).__init__(NodeType.REGULAR_NODE,
                                          name, "nimiq-client", listen_port,
                                          topology_settings, sync_mode, rpc,
                                          metrics, container_image)

    def generate_config_files(self, jinja_env: Environment,
                              listen_ip: str, seed_addresses: list):
        """
        Generates configuration file

        :param jinja_env: Jinja2 environment for template rendering
        :type jinja_env: Environment
        :param listen_ip: Ip for the node where incoming connections are going
            to be listened.
        :type listen_ip: str
        :param seed_addresses: List of seed addresses in multiaddress format
            for the configuration file
        :type seed_addresses: List of strings
        """
        filename = self.get_conf_toml()
        content = self.get_config_files_content(jinja_env, listen_ip,
                                                seed_addresses)
        with open(filename, mode="w", encoding="utf-8") as message:
            message.write(content)

    def generate_k8s_file(self, jinja_env: Environment,
                          listen_ip: str, seed_addresses: list):
        """
        Generates the k8s manifest file

        :param jinja_env: Jinja2 environment for template rendering
        :type jinja_env: Environment
        :param listen_ip: Ip for the node where incoming connections are going
            to be listened.
        :type listen_ip: str
        :param seed_addresses: List of seed addresses in multiaddress format
            for the configuration file
        :type seed_addresses: List of strings
        """
        int_genesis_dir = "/home/nimiq/genesis"
        genesis_filename = "dev-albatross.toml"
        int_genesis_file = f"{int_genesis_dir}/{genesis_filename}"
        filename = self.topology_settings.get_node_k8s_dir(self.name)
        namespace = self.topology_settings.get_namespace()
        enable_metrics = self.get_metrics() is not None
        config_content = self.get_config_files_content(jinja_env, listen_ip,
                                                       seed_addresses)
        # Now read and render the template
        template = jinja_env.get_template("k8s_node_deployment.yml.j2")
        content = template.render(name=self.get_name(),
                                  node_type='regular_node',
                                  namespace=namespace,
                                  internal_genesis_file=int_genesis_file,
                                  internal_genesis_dir=int_genesis_dir,
                                  genesis_filename=genesis_filename,
                                  config_content=config_content,
                                  enable_metrics=enable_metrics,
                                  container_image=self.container_image)
        with open(filename, mode="w", encoding="utf-8") as file:
            file.write(content)

    def get_config_files_content(self, jinja_env: Environment, listen_ip: str,
                                 seed_addresses: list):
        """
        Gets the configuration content

        :param jinja_env: Jinja2 environment for template rendering
        :type jinja_env: Environment
        :param listen_ip: Ip for the node where incoming connections are going
            to be listened.
        :type listen_ip: str
        :param seed_addresses: List of seed addresses in multiaddress format
            for the configuration file
        :type seed_addresses: List of strings
        :return: The configuration file content
        :rtype: str
        """
        # Read and render the TOML template
        template = jinja_env.get_template("node_conf.toml.j2")
        rpc = self.get_rpc()
        metrics = self.get_metrics()
        loki_settings = self.topology_settings.get_loki_settings()
        if loki_settings is not None:
            loki_settings = loki_settings.format_for_config_file()
            loki_settings['extra_fields']['nimiq_node'] = self.name
        content = template.render(
            min_peers=3, port=self.get_listen_port(),
            state_path=self.get_state_dir(), listen_ip=listen_ip,
            sync_mode=self.get_sync_mode(), seed_addresses=seed_addresses,
            rpc=rpc, metrics=metrics, loki=loki_settings)
        return content


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
    :param rpc: Optional rpc settings
    :type rpc: Optional[dict]
    :param metrics: Optional metrics settings
    :type metrics: Optional[dict]
    :param container_image: Optional container image
    :type container_image: Optional[str]
    """

    def __init__(self, name: str, listen_port: int,
                 topology_settings: TopologySettings, sync_mode: str = "full",
                 rpc: Optional[dict] = None, metrics: Optional[dict] = None,
                 container_image: Optional[str] = None):
        super(Seed, self).__init__(NodeType.SEED,
                                   name, "nimiq-client", listen_port,
                                   topology_settings, sync_mode, rpc,
                                   metrics, container_image)

    def generate_config_files(self, jinja_env: Environment, listen_ip: str):
        """
        Generates configuration file

        :param jinja_env: Jinja2 environment for template rendering
        :type jinja_env: Environment
        :param listen_ip: Ip for the node where incoming connections are going
            to be listened.
        :type listen_ip: str
        """
        filename = self.get_conf_toml()
        content = self.get_config_files_content(jinja_env, listen_ip)
        with open(filename, mode="w", encoding="utf-8") as message:
            message.write(content)

    def generate_k8s_file(self, jinja_env: Environment, listen_ip: str):
        """
        Generates the k8s manifest file

        :param jinja_env: Jinja2 environment for template rendering
        :type jinja_env: Environment
        :param listen_ip: Ip for the node where incoming connections are going
            to be listened.
        :type listen_ip: str
        """
        int_genesis_dir = "/home/nimiq/genesis"
        genesis_filename = "dev-albatross.toml"
        int_genesis_file = f"{int_genesis_dir}/{genesis_filename}"
        filename = self.topology_settings.get_node_k8s_dir(self.name)
        namespace = self.topology_settings.get_namespace()
        enable_rpc = self.get_rpc() is not None
        enable_metrics = self.get_metrics() is not None
        config_content = self.get_config_files_content(jinja_env, listen_ip)
        # Now read and render the template
        template = jinja_env.get_template("k8s_node_deployment.yml.j2")
        content = template.render(name=self.get_name(),
                                  node_type='seed',
                                  namespace=namespace,
                                  internal_genesis_file=int_genesis_file,
                                  internal_genesis_dir=int_genesis_dir,
                                  genesis_filename=genesis_filename,
                                  config_content=config_content,
                                  enable_rpc=enable_rpc,
                                  enable_metrics=enable_metrics,
                                  container_image=self.container_image)
        with open(filename, mode="w", encoding="utf-8") as file:
            file.write(content)

    def get_config_files_content(self, jinja_env: Environment, listen_ip: str):
        """
        Gets the configuration file content

        :param jinja_env: Jinja2 environment for template rendering
        :type jinja_env: Environment
        :param listen_ip: Ip for the node where incoming connections are going
            to be listened.
        :type listen_ip: str
        :return: The configuration file content
        :rtype: str
        """
        # Read and render the TOML template
        template = jinja_env.get_template("node_conf.toml.j2")
        rpc = self.get_rpc()
        metrics = self.get_metrics()
        loki_settings = self.topology_settings.get_loki_settings()
        if loki_settings is not None:
            loki_settings = loki_settings.format_for_config_file()
            loki_settings['extra_fields']['nimiq_node'] = self.name
        content = template.render(
            min_peers=3, port=self.get_listen_port(),
            state_path=self.get_state_dir(), listen_ip=listen_ip,
            sync_mode=self.get_sync_mode(), rpc=rpc, metrics=metrics,
            loki=loki_settings)
        return content
