from typing import Optional


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
    :type nimiq_exec: str
    :param release: Flag to indicate whether to run in release mode or not.
    :type release: bool
    :param logs_dir: Directory containing the logs.
    :type logs_dir: str
    :param conf_dir: Directory containing the configuration files.
    :type conf_dir: str
    :param state_dir: Directory where the node state will be stored.
    :type state_dir: str
    """

    def __init__(self, nimiq_dir: str, logs_dir: str, conf_dir: str,
                 state_dir: str, release: bool,
                 loki_settings: Optional[LokiSettings] = None):
        self.nimiq_dir = nimiq_dir
        self.logs_dir = logs_dir
        self.conf_dir = conf_dir
        self.state_dir = state_dir
        self.release = release
        self.loki_settings = loki_settings

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
        Gets the directory containing the configuration files

        :return: The path of the directory of the configuration files
        :rtype: str
        """
        return self.conf_dir

    def get_state_dir(self):
        """
        Gets the directory containing the node state related files

        :return: The path of the directory of the node state related files
        :rtype: str
        """
        return self.state_dir

    def get_release(self):
        """
        Gets release flag

        :return: The release flag
        :rtype: bool
        """
        return self.release

    def get_loki_settings(self):
        """
        Gets the Loki settings

        :return: The Loki settings
        :rtype: Optional[LokiSettings]
        """
        return self.loki_settings
