from typing import Optional
from jinja2 import Environment

from node import Node, NodeType
from topology_settings import TopologySettings


class Spammer(Node):
    """
    Devnet spammer node

    :param name: Name of the validator node.
    :type name: str
    :param listen_port: Port this node will be listening to connections.
    :type listen_port: int
    :param topology_settings: General topology settings
    :type topology_settings: TopologySettings
    :param profile: Path to the spammer profile that should be used
    :type profile: str
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
                 topology_settings: TopologySettings, profile: str,
                 sync_mode: str = 'history', rpc: Optional[dict] = None,
                 metrics: Optional[dict] = None,
                 container_image: Optional[str] = None):
        self.address = "NQ40 GCAA U3UX 8BKD GUN0 PG3T 17HA 4X5H TXVE"
        self.profile = profile

        nimiq_exec_extra_args = ['--profile', profile]

        super(Spammer, self).__init__(NodeType.SPAMMER,
                                      name, "nimiq-spammer", listen_port,
                                      topology_settings, sync_mode,
                                      rpc, metrics, container_image,
                                      nimiq_exec_extra_args)

    def get_address(self):
        """
        Gets the spammer address

        :return: The validator address keypair.
        :rtype: str
        """
        return self.address

    def get_profile(self):
        """
        Gets the spammer profile

        :return: Path to the spammer profile
        :rtype: str
        """
        return self.profile

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
        :type seed_addresses: List of strings
        """
        filename = self.get_conf_toml()
        content = self.get_config_files_content(jinja_env, listen_ip,
                                                seed_addresses)
        with open(filename, mode="w", encoding="utf-8") as message:
            message.write(content)

    def generate_k8s_file(self, jinja_env: Environment, listen_ip: str,
                          seed_addresses: list):
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
        enable_rpc = self.get_rpc() is not None
        enable_metrics = self.get_metrics() is not None
        config_content = self.get_config_files_content(jinja_env, listen_ip,
                                                       seed_addresses)
        # Now read and render the template
        template = jinja_env.get_template("k8s_node_deployment.yml.j2")
        content = template.render(name=self.get_name(),
                                  node_type='spammer',
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

    def get_config_files_content(self, jinja_env: Environment, listen_ip: str,
                                 seed_addresses: list):
        """
        Gets the configuration file content

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
            rpc=rpc, metrics=metrics, spammer=True, loki=loki_settings)
        return content
