from typing import Optional


class RestartSettings:
    """
    Restart settings

    :param up_time: Time in seconds during which restartable nodes are up.
    :type up_time: int
    :param down_time: Time in seconds that restartable nodes are taken down.
    :type down_time: int
    :param max_restarts: The number of times you want to kill/restart node.
    :type max_restarts: int
    :param sim_kills: How many simultaneous nodes are killed each cycle.
    :type sim_kills: int
    :param erase_db: Flag to erase all of the node state as part of restarting
        it.
    :type erase_db: bool
    :param erase_state: Flag to erase only the node state of the validator as
        part of restarting it.
    :type erase_state: bool
    """

    def __init__(self, up_time: int, down_time: int, max_restarts: int,
                 sim_kills: int, erase_db: bool = False,
                 erase_state: bool = False):
        self.up_time = up_time
        self.down_time = down_time
        self.max_restarts = max_restarts
        self.sim_kills = sim_kills
        self.erase_db = erase_db
        self.erase_state = erase_state

    def get_up_time(self):
        """
        Gets the time in seconds during which all nodes are up.

        :return: The uptime value.
        :rtype: int
        """
        return self.up_time

    def get_down_time(self):
        """
        Gets the time in seconds that nodes are taken down.

        :return: The downtime value.
        :rtype: int
        """
        return self.down_time

    def get_max_restarts(self):
        """
        Gets the number of times you want to kill/restart nodes

        :return: The number of restarts.
        :rtype: int
        """
        return self.max_restarts

    def get_sim_kills(self):
        """
        Gets how many simultaneous nodes are killed each cycle

        :return: The number of restarts.
        :rtype: int
        """
        return self.sim_kills

    def get_erase_db(self):
        """
        Gets the flag that indicates whether to erase all of the node state as
        part of restarting it.

        :return: The erase DB flag.
        :rtype: bool
        """
        return self.erase_db

    def get_erase_state(self):
        """
        Gets the flag that indicates whether to erase only the database state of
        the node as part of restarting it.

        :return: The erase state flag.
        :rtype: bool
        """
        return self.erase_state


class ControlSettings:
    """
    General topology settings

    :param restart_settings: Optional restart settings. Setting this to None is
        equivalent of running in continuous mode. If this argument isn't
        passed continuous mode is assumed and `monitor_interval` is required.
    :type restart_settings: Optional[RestartSettings]
    :param monitor_interval: Optional integer specifying for how long nodes
        should freely run before monitoring if they panicked or stopped
        producing blocks. This only takes effect in continuous mode (not
        passing `restart_settings`).
    :type monitor_interval: Optiona[int]
    """

    def __init__(self, restart_settings: Optional[RestartSettings] = None,
                 monitor_interval: Optional[int] = None):
        if restart_settings is None and monitor_interval is None:
            raise Exception("Either restart settings or monitor interval "
                            "should be passed")
        self.restart_settings = restart_settings
        self.monitor_interval = monitor_interval

    def is_continuous(self):
        """
        Wether the control settings were configured for continuous mode or not

        :return: A boolean indicating the continuous mode configuration.
        :rtype: bool
        """
        return self.restart_settings is None

    def get_restart_settings(self):
        """
        Gets the restart settings.

        :return: The restart settings
        :rtype: Optional[RestartSettings]
        """
        return self.restart_settings

    def get_monitor_interval(self):
        """
        Gets the monitor interval setting.

        :return: The monitor interval setting
        :rtype: Optional[int]
        """
        return self.monitor_interval
