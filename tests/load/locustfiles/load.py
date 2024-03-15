# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at http://mozilla.org/MPL/2.0/.

"""Load test shape module."""
import math
import os
from typing import Type

import numpy
from locust import LoadTestShape, User
from locustfile import AutopushUser
from stored import StoredNotifAutopushUser

TickTuple = tuple[int, float, list[Type[User]]]


class QuadraticTrend:
    """A class that defines a quadratic LoadTestShape trend."""

    # variables correspond to the quadratic function: f(x) = ax^2 + bx + c
    a: float
    b: float
    c: float

    def __init__(self, max_run_time: int, max_users: int):
        self.a, self.b, self.c = numpy.polyfit(
            [0, (max_run_time / 2), max_run_time], [0, max_users, 0], 2
        )

    def calculate_users(self, run_time: int) -> int:
        """Determine the number of active users given a run time.

        Parameters
        ----------
        run_time : int
            Run time in seconds

        Returns
        -------
        run_time : int
            The number of users
        """
        return int(round((self.a * math.pow(run_time, 2)) + (self.b * run_time) + self.c))


class AutopushLoadTestShape(LoadTestShape):
    """A load test shape class for Autopush (Duration: 10 minutes, Users: 150000).

    Note: The Shape class assumes that the workers can support the generated spawn rates. Should
    the number of available Locust workers change or should the Locust worker capacity change,
    the WORKERS_COUNT and USERS_PER_WORKER values must be changed respectively.
    """

    MAX_RUN_TIME: int = int(os.environ.get("MAX_RUN_TIME", 600))  # 10 minutes
    WORKER_COUNT: int = int(
        os.environ.get("WORKER_COUNT", 150)
    )  # Must match value defined in setup_k8s.sh
    USERS_PER_WORKER: int = int(
        os.environ.get("USERS_PER_WORKER", 1000)
    )  # Number of users supported on a worker running on a n1-standard-2
    MAX_USERS: int = WORKER_COUNT * USERS_PER_WORKER
    trend: QuadraticTrend
    user_classes: list[Type[User]] = [AutopushUser]

    def __init__(self):
        super(LoadTestShape, self).__init__()
        self.trend = QuadraticTrend(self.MAX_RUN_TIME, self.MAX_USERS)

    def tick(self) -> TickTuple | None:
        """Override defining the desired distribution for Autopush load testing.

        Returns
        -------
        TickTuple | None

        TickTuple Contained Parameters
            user_count: Total user count
            spawn_rate: Number of users to start/stop per second when changing
                        number of users
            user_classes: None or a List of user classes to be spawned

        None: Instruction to stop the load test
        """
        run_time: int = int(self.get_run_time())
        if run_time > self.MAX_RUN_TIME:
            return None

        users: int = self.trend.calculate_users(run_time)
        # The spawn_rate minimum value is 1
        spawn_rate: float = max(abs(users - self.get_current_user_count()), 1)

        return users, spawn_rate, self.user_classes


class StoredNotifAutopushLoadTestShape(AutopushLoadTestShape):
    """A load test shape class for StoredNotifAutopushLoadTestShape
    (Duration: 15 minutes, Users: 150000).
    """

    MAX_RUN_TIME: int = 900  # 15 minutes
    user_classes: list[Type[User]] = [StoredNotifAutopushUser]
