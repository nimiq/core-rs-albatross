from typing import Optional
from enum import Enum


class Environment(Enum):
    """
    Types of environments supported by the topology
    """
    LOCAL = 0
    """Topology that will run in a local environment using processes and
     different node ports"""
    DOCKER_COMPOSE = 1
    """Topology that will run in a docker compose environment"""
    K8S = 2
    """Topology that will run in a Kubernetes environment"""

    def __str__(self):
        return self.name.lower()

    @staticmethod
    def from_string(s: str):
        try:
            return Environment[s.upper()]
        except KeyError:
            raise ValueError()


class LokiSettings:
    """
    Topology Loki settings

    :param url: Loki URL.
    :type url: str
    :param labels: Loki labels.
    :type labels: dict
    :param extra_fields: Loki extra fields.
    :type extra_fields: dict
    """

    def __init__(self, url: str, labels: dict, extra_fields: dict):
        self.url = url
        self.labels = labels
        self.extra_fields = extra_fields

    def get_url(self):
        """
        Gets the Loki URL.

        :return: The Loki URL.
        :rtype: str
        """
        return self.url

    def get_labels(self):
        """
        Gets the Loki labels.

        :return: The Loki labels
        :rtype: dict
        """
        return self.labels

    def get_extra_fields(self):
        """
        Gets the Loki extra fields.

        :return: The Loki extra fields
        :rtype: dict
        """
        return self.extra_fields

    def format_for_config_file(self):
        """
        Gets the Loki settings formatted in a dictionary to be used in node
        configuration files.

        :return: The loki settings
        :rtype: dict
        """
        formatted = dict()
        formatted['url'] = self.url
        formatted['labels'] = self.labels.copy()
        formatted['extra_fields'] = self.extra_fields.copy()
        return formatted


class TopologySettings:
    """
    General topology settings

    :param nimiq_dir: Directory of the Nimiq repository.
    :type nimiq_dir: str
    :param logs_dir: Directory containing the logs.
    :type logs_dir: str
    :param conf_dir: Directory containing the configuration files.
    :type conf_dir: str
    :param state_dir: Directory where the node state will be stored.
    :type state_dir: str
    :param release: Flag to indicate whether to run in release mode or not.
    :type release: bool
    :param namespace: k8s namespace for the generated resources.
    :type namespace: str
    :param env: Type of environment this topology will be run in.
    :type env: Environment
    :param loki_settings: Optional Loki settings.
    :type loki_settings: Optional[LokiSettings]
    """

    def __init__(
            self, nimiq_dir: str, logs_dir: str, conf_dir: str, state_dir: str,
            release: bool, namespace: str, env: Environment,
            loki_settings: Optional[LokiSettings] = None):
        self.nimiq_dir = nimiq_dir
        self.logs_dir = logs_dir
        self.conf_dir = conf_dir
        self.state_dir = state_dir
        self.release = release
        self.namespace = namespace
        self.loki_settings = loki_settings
        self.env = env

    def get_nimiq_dir(self):
        """
        Gets the directory containing the log files

        :return: The path of the directory of the log files
        :rtype: str
        """
        return self.nimiq_dir

    def get_logs_dir(self):
        """
        Gets the directory of the Nimiq repository

        :return: The path of the directory of the Nimiq repository
        :rtype: str
        """
        return self.logs_dir

    def get_conf_dir(self):
        """
        Gets the directory containing the all node's configuration files.
        This directory will include other directories with each node
        configuration.

        :return: The path of the directory of the configuration files
        :rtype: str
        """
        return self.conf_dir

    def get_state_dir(self):
        """
        Gets the directory containing all node's state related files.
        In non containerized topologies, this directory will include other
        directories with each node state related files.

        :return: The path of the directory of all node's state related files
        :rtype: str
        """
        return self.state_dir

    def get_k8s_dir(self):
        """
        Gets the directory containing all node's k8s manifest files

        :return: The path of the directory of all node's k8s manifest files
        :rtype: str
        """
        if self.env != Environment.K8S:
            raise Exception("Requested k8s settings for a non k8s environment")
        return f"{self.conf_dir}/k8s"

    def get_node_conf_dir(self, node_name: str):
        """
        Gets the directory containing the node configuration files

        :param name: Name of the node.
        :type name: str
        :return: The path of the directory of the configuration files
        :rtype: str
        """
        return f"{self.conf_dir}/{node_name}"

    def get_node_state_dir(self, node_name: str):
        """
        Gets the directory containing the node state related files

        :param name: Name of the node.
        :type name: str
        :return: The path of the directory of the node state related files
        :rtype: str
        """
        # In containerized topologies, the state can be the same for all
        # nodes since they are isolated.
        if self.is_env_containerized():
            return self.state_dir
        else:
            return f"{self.state_dir}/{node_name}"

    def get_node_k8s_dir(self, node_name: str):
        """
        Gets the directory containing the node k8s manifest files

        :param name: Name of the node.
        :type name: str
        :return: The path of the directory of the node state related files
        :rtype: str
        """
        return f"{self.get_k8s_dir()}/{node_name}.yml"

    def get_release(self):
        """
        Gets release flag

        :return: The release flag
        :rtype: bool
        """
        return self.release

    def get_namespace(self):
        """
        Gets k8s namespace

        :return: The k8s namespace
        :rtype: str
        """
        return self.namespace

    def get_env(self):
        """
        Gets the environment of this topology

        :return: The environment this topology will be run in
        :rtype: Environment
        """
        return self.env

    def is_env_containerized(self):
        """
        Returns whether the environment is containerized (docker-compose or
        k8s)

        :returns: A flag telling if the environment corresponds to a
            containerized one
        :rtype: bool
        """
        return self.env in [Environment.K8S, Environment.DOCKER_COMPOSE]

    def get_loki_settings(self):
        """
        Gets the Loki settings

        :return: The Loki settings
        :rtype: Optional[LokiSettings]
        """
        return self.loki_settings
