from typing import Optional
from jinja2 import Environment

from node import Node, NodeType
from topology_settings import TopologySettings
from utils import create_bls_keypair, create_schnorr_keypair


class Validator(Node):
    """
    Devnet validator node

    :param name: Name of the validator node.
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
        if sync_mode == "light":
            raise Exception("Validator can't use light sync_mode")
        self.voting_keypair = create_bls_keypair(topology_settings)
        self.signing_keypair = create_schnorr_keypair(topology_settings)
        self.address = create_schnorr_keypair(topology_settings)
        self.reward_address = create_schnorr_keypair(topology_settings)
        super(Validator, self).__init__(NodeType.VALIDATOR,
                                        name, "nimiq-client", listen_port,
                                        topology_settings, sync_mode, metrics)

    def get_voting_keypair(self):
        """
        Gets the validator voting keypair

        :return: The validator voting keypair.
        :rtype: dict
        """
        return self.voting_keypair

    def get_signing_keypair(self):
        """
        Gets the validator signing keypair

        :return: The validator signing keypair.
        :rtype: dict
        """
        return self.signing_keypair

    def get_address_keypair(self):
        """
        Gets the validator address keypair

        :return: The validator address keypair.
        :rtype: dict
        """
        return self.address

    def get_reward_address_keypair(self):
        """
        Gets the validator reward address keypair

        :return: The validator reward address keypair.
        :rtype: dict
        """
        return self.reward_address

    def generate_config_files(self, jinja_env: Environment, listen_ip: str,
                              seed_addresses: list):
        """
        Generates configuration file

        :param jinja_env: Jinja2 environment for template rendering
        :type jinja_env: Environment
        :param listen_ip: Ip for the node where incoming connections are going
            to be listened.
        :type listen_ip: str
        :param seed_addresses: List of seed addresses in multiaddress format
            for the configuration file
        :type seed_addresses: List of ints
        """
        # Read and render the TOML template
        template = jinja_env.get_template("node_conf.toml.j2")
        data = dict()
        data['validator_address'] = self.get_address_keypair()[
            'address']
        data['signing_key'] = self.get_signing_keypair()[
            'private_key']
        data['voting_key'] = self.get_voting_keypair()['private_key']
        data['fee_key'] = self.get_reward_address_keypair()[
            'private_key']
        metrics = self.get_metrics()
        loki_settings = self.topology_settings.get_loki_settings()
        if loki_settings is not None:
            loki_settings = loki_settings.format_for_config_file()
            loki_settings['extra_fields']['nimiq_node'] = self.name
        if metrics is not None:
            content = template.render(
                min_peers=3, port=self.get_listen_port(),
                state_path=self.get_state_dir(), listen_ip=listen_ip,
                sync_mode=self.get_sync_mode(), validator=data,
                seed_addresses=seed_addresses, metrics=metrics,
                loki=loki_settings)
        else:
            content = template.render(
                min_peers=3, port=self.get_listen_port(),
                state_path=self.get_state_dir(), listen_ip=listen_ip,
                sync_mode=self.get_sync_mode(), validator=data,
                seed_addresses=seed_addresses, loki=loki_settings)
        filename = self.get_conf_toml()
        with open(filename, mode="w", encoding="utf-8") as message:
            message.write(content)

        # Read and render the YAML template
        template = jinja_env.get_template("ansible.yml.j2")
        content = template.render(validator=data)
        filename = self.get_conf_yaml()
        with open(filename, mode="w", encoding="utf-8") as message:
            message.write(content)
