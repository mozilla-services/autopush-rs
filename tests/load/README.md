# Autopush Load (Locust) Tests

This directory contains the automated load test suite for autopush. These tests are run
using a tool named [locust][1].

## Related Documentation

* [Autopush Load Test Class Diagram][2]
* [Autopush Load Test Spreadsheet][3]
* [Autopush Load Test Artifacts][4]

## Contributing

This project uses [Poetry][5] for dependency management. For environment setup it is
recommended to use [pyenv][6] and [pyenv-virtualenv][7], as they work nicely with
Poetry.

Project dependencies are listed in the `pyproject.toml` file.
To install the dependencies execute from the `tests` directory:

```shell
poetry install
```

Contributors to this project are expected to execute [isort][8], [black][9], [flake8][10]
and [mypy][11] for import sorting, linting, style guide enforcement and static type
checking respectively. Configurations are set in the `pyproject.toml` and `.flake8`
files.

The tools can be executed with the following command from the root directory:

```shell
make lint
```

## Local Execution

Follow the steps bellow to execute the load tests locally:

### Setup Environment

#### 1. Configure Environment Variables

Environment variables, listed bellow or specified by [Locust][12], can be set in
`tests\load\docker-compose.yml`.

| Environment Variable | Node(s)         | Description                          |
|----------------------|-----------------|--------------------------------------|
| AUTOPUSH_WAIT_TIME   | master & worker | The wait time between task execution |
| AUTOPUSH_VAPID_KEY   | worker          | path to the VAPID private key        |

#### 2. Host Locust via Docker

Execute the following from the root directory:

```shell
make load
```

### Run Test Session

#### 1. Start Load Test

* In a browser navigate to `http://localhost:8089/`
* Set up the load test parameters:
    * ShapeClass: Default
    * Number of users: 1
    * Ramp up: 1
    * Host: 'wss://autoconnect.stage.mozaws.net'
    * Duration (Optional): 10m
* Select "Start Swarm"

#### 2. Stop Load Test

Select the 'Stop' button in the top right hand corner of the Locust UI, after the
desired test duration has elapsed. If the 'Run time' or 'Duration' is set in step 1,
the load test will stop automatically.

#### 3. Analyse Results

* See [Distributed GCP Execution - Analyse Results](#3-analyse-results-1)
* Only client-side measures, provided by Locust, are available for local execution

### Clean-up Environment

#### 1. Remove Load Test Docker Containers

Execute the following from the root directory:

```shell
make load-clean
```

## Distributed GCP Execution

Follow the steps bellow to execute the distributed load tests on GCP:

### Setup Environment

#### 1. Start a GCP Cloud Shell

The load tests can be executed from the [contextual-services-test-eng cloud shell][15].
If executing a load test for the first time, the git autopush repository will need to
be cloned locally into the root directory of the machine. Use the HTTPS git clone
functionality as SSH is not supported in this environment.

#### 2. Configure the Bash Script

* The `setup_k8s.sh` file, located in the `tests\load` directory, contains
  shell commands to **create** a GKE cluster, **setup** an existing GKE cluster or
  **delete** a GKE cluster
    * Execute the following from the root directory, to make the file executable:
      ```shell
      chmod +x tests/load/setup_k8s.sh
      ```

#### 3. Create the GCP Cluster

* Execute the `setup_k8s.sh` file from the root directory and select the **create**
  option, in order to initiate the process of creating a cluster, setting up the env
  variables and building the docker image
  ```shell
  ./tests/load/setup_k8s.sh create
  ```
  **Note:** Locust requires CPU affinity to execute most efficiently and can only make
  use of a single core at a time. Warnings will be logged if CPU usage exceeds 90%.
* The cluster creation process will take some time. It is considered complete, once
  an external IP is assigned to the `locust_master` node. Monitor the assignment via
  a watch loop:
  ```bash
  kubectl get svc locust-master --watch
  ```
* The number of workers is defaulted to 100, but can be modified with the
  `kubectl scale` command. Example (10 workers):
  ```bash
  kubectl scale deployment/locust-worker --replicas=10
  ```
* To apply new changes to an existing GCP Cluster, execute the `setup_k8s.sh` file and
  select the **setup** option.
    * This option will consider the local commit history, creating new containers and
      deploying them (see [Artifact Registry][16])

### Run Test Session

#### 1. Start Load Test

* In a browser navigate to `http://$EXTERNAL_IP:8089`
  This url can be generated via command
  ```bash
  EXTERNAL_IP=$(kubectl get svc locust-master -o jsonpath="{.status.loadBalancer.ingress[0].ip}")
  echo http://$EXTERNAL_IP:8089
  ```
* Set up the load test parameters:
    * ShapeClass: 'AutopushLoadTestShape'
    * Host: 'wss://autoconnect.stage.mozaws.net'
* Select "Start Swarm"

#### 2. Stop Load Test

Select the 'Stop' button in the top right hand corner of the Locust UI, after the
desired test duration has elapsed. If the 'Run time' or 'Duration' is set in step 1,
the load test will stop automatically.

#### 3. Analyse Results

**Failures**

* The number of responses with errors (non-200 response codes) should be `0`
* Locust reports Failures via the "autopush_failures.csv" file and the UI
  (under the "Failures" tab or the "Charts" tab)
* [Grafana][17] reports Failures via the "HTTP Response codes" chart and the
  "HTTP 5xx error rate" chart

**Exceptions**

* Exceptions indicate errors that occur during Locust's execution of the load tests and
  should be minimal.
* The following exceptions are known to happen, but make sure their occurrence isn't
  trending positively:
    * ZeroStatusRequestError
* Locust reports Exceptions via the "autopush_exceptions.csv" file and the UI
  (under the "Exceptions" tab)

#### 4. Report Results

* Results should be recorded in the [Autopush Load Test Spreadsheet][3]
* Optionally, the Locust reports can be saved and linked in the spreadsheet:
    * Download the results via the Locust UI, under the 'Charts' and 'Download Data'
      tabs or via command:
        ```bash
        kubectl cp <master-pod-name>:/home/locust/autopush_stats.csv autopush_stats.csv
        kubectl cp <master-pod-name>:/home/locust/autopush_exceptions.csv autopush_exceptions.csv
        kubectl cp <master-pod-name>:/home/locust/autopush_failures.csv autopush_failures.csv
        kubectl cp <master-pod-name>:/home/locust/autopush.log autopush.log
        ```
      The `master-pod-name` can be found at the top of the pod list:
        ```bash
        kubectl get pods -o wide
        ```
    * Upload the files to the [ConServ][4] drive and record the links in the
      spreadsheet

### Clean-up Environment

#### 1. Delete the GCP Cluster

Execute the `setup_k8s.sh` file from the root directory and select the **delete** option

```shell
./tests/load/setup_k8s.sh delete
```

## Calibration

Following the addition of new features, such as a Locust Task or Locust User, or
environmental changes, such as node size or the upgrade of a major dependency like the
python version image, it may be necessary to re-establish the recommended parameters of
the performance test.

| Parameter         | Description                                                                                                                                                                                                      |
|-------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| [WAIT TIME][13]   | - Changing this cadence will increase or decrease the number of channel subscriptions and notifications sent by an AutopushUser. <br/>- This value can be set in the Locust UI.                                  |
| [TASK WEIGHT][14] | - Changing this weight impacts the probability of a task being chosen for execution. <br/>- This value is hardcoded in the task decorators of the AutopushUser class.                                            |
| `USERS_PER_WORKER`  | - This value should be set to the maximum number of users a Locust worker can support given CPU and memory constraints. <br/>- This value is hardcoded in the AutopushLoadTestShape class.                       |
| `WORKER_COUNT`      | - This value is derived by dividing the total number of users needed for the performance test by the `USERS_PER_WORKER`. <br>- This value is hardcoded in the AutopushLoadTestShape and the setup_k8s.sh script. |

## Calibrating for USERS_PER_WORKER

This process is used to determine the number of users that a Locust worker can support.

### Setup Environment

#### 1. Start a GCP Cloud Shell

The load tests can be executed from the [contextual-services-test-eng cloud shell][15].
If executing a load test for the first time, the git autopush repository will need to
be cloned locally.

#### 2. Configure the Bash Script

* The `setup_k8s.sh` file, located in the `tests\load` directory, contains
  shell commands to **create** a GKE cluster, **setup** an existing GKE cluster or
  **delete** a GKE cluster
    * Execute the following from the root directory, to make the file executable:
      ```shell
      chmod +x tests/load/setup_k8s.sh
      ```

#### 3. Create the GCP Cluster

* In the `setup_k8s.sh` script, modify the `WORKER_COUNT` variable to equal `1`
* Execute the `setup_k8s.sh` file from the root directory and select the **create**
  option, in order to initiate the process of creating a cluster, setting up the env
  variables and building the docker image
  ```shell
  ./tests/load/setup_k8s.sh create
  ```
* The cluster creation process will take some time. It is considered complete, once
  an external IP is assigned to the `locust_master` node. Monitor the assignment via
  a watch loop:
  ```bash
  kubectl get svc locust-master --watch
  ```

### Calibrate

Repeat steps 1 to 3, using a process of elimination, such as the bisection method, to
determine the maximum `USERS_PER_WORKER`. The load tests are considered optimized when
CPU and memory resources are maximally utilized. This step is meant to determine the
maximum user count that a node can accommodate by observing CPU and memory usage while
steadily increasing or decreasing the user count. You can monitor the CPU percentage in
the Locust UI but also in the Kubernetes engine Workloads tab where both memory and CPU
are visualized on charts.

#### 1. Start Load Test

* In a browser navigate to `http://$EXTERNAL_IP:8089`
  This url can be generated via command
  ```bash
  EXTERNAL_IP=$(kubectl get svc locust-master -o jsonpath="{.status.loadBalancer.ingress[0].ip}")
  echo http://$EXTERNAL_IP:8089
  ```
* Set up the load test parameters:
    * ShapeClass: Default
    * UserClasses: StoredNotifAutopushUser (This is considered the more resource hungry user)
    * Number of users: USERS_PER_WORKER (Consult the [Autopush Load Test Spreadsheet][3] to determine a starting point)
    * Ramp up: RAMP_UP (RAMP_UP = 600/USERS_PER_WORKER)
    * Host: `wss://autoconnect.stage.mozaws.net`
    * Duration (Optional): 600s
* Select "Start Swarm"

#### 2. Stop Load Test

Select the 'Stop' button in the top right hand corner of the Locust UI, after the
desired test duration has elapsed. If the 'Run time' or 'Duration' is set in step 1,
the load test will stop automatically.

#### 3. Analyse Results

**CPU and Memory Resource Graphs**

* CPU and Memory usage should be less than 90% of the available capacity
    * CPU and Memory Resources can be observed in
      [Google Cloud > Kubernetes Engine > Workloads][18]

**Log Errors or Warnings**

* Locust will emit errors or warnings if high CPU or memory usage occurs during the
  execution of a load test. The presence of these logs is a strong indication that the
  `USERS_PER_WORKER` count is too high
    * Errors and Warnings emitted while Locust is stopping can be ignored. This can be
      caused by disconnecting too many websockets at once, which doesn't happen in
      production because of the use of Shape classes

#### 4. Report Results

* See [Distributed GCP Execution - Report Results](#4-report-results-1)

#### 5. Update Shape and Script Values

* `WORKER_COUNT = MAX_USERS/USERS_PER_WORKER`
    * If `MAX_USERS` is unknown, calibrate to determine `WORKER_COUNT`
* Update the `USERS_PER_WORKER` and `WORKER_COUNT` values in the following files:
    * `\tests\load\locustfiles\load.py`
    * \tests\load\setup_k8s.sh

### Clean-up Environment

#### 1. Delete the GCP Cluster

* See [Distributed GCP Execution - Clean-up Environment](#clean-up-environment)

## Calibrating for WORKER_COUNT

This process is used to determine the number of Locust workers required in order to
generate sufficient load for a test given a SHAPE_CLASS.

### Setup Environment

* See [Distributed GCP Execution - Setup Environment](#setup-environment-1)
* Note that in the `setup_k8s.sh` the maximum number of nodes is set using the
  `total-max-nodes` google cloud option. It may need to be increased if the number of
  workers can't be supported by the cluster.

### Calibrate

Repeat steps 1 to 4, using a process of elimination, such as the bisection method, to
determine the maximum `WORKER_COUNT`. The tests are considered optimized when they
generate the minimum load required to cause node scaling in the the autopush Stage
environment. You can monitor the autopush pod counts on [Grafana][17].

#### 1. Update Shape and Script Values

* Update the `WORKER_COUNT` values in the following files:
    * `\tests\load\locustfiles\load.py`
    * \tests\load\setup_k8s.sh
* Using Git, commit the changes locally

#### 2. Start Load Test

* In a browser navigate to `http://$EXTERNAL_IP:8089`
  This url can be generated via command
  ```bash
  EXTERNAL_IP=$(kubectl get svc locust-master -o jsonpath="{.status.loadBalancer.ingress[0].ip}")
  echo http://$EXTERNAL_IP:8089
  ```
* Set up the load test parameters:
    * ShapeClass: SHAPE_CLASS
    * Host: 'wss://autoconnect.stage.mozaws.net'
* Select "Start Swarm"

#### 3. Stop Load Test

Select the 'Stop' button in the top right hand corner of the Locust UI, after the
desired test duration has elapsed. If the 'Run time', 'Duration' or 'ShapeClass' is set
are set in step 1, the load test will stop automatically.

#### 4. Analyse Results

**Stage Environment Pod Counts**

* The 'Autoendpoint Pod Count' or 'Autoconnect Pod Count' should demonstrate scaling
  during the execution of the load test
  * The pod counts can be observed in [Grafana][17]

**CPU and Memory Resources**

* CPU and Memory usage should be less than 90% of the available capacity in the cluster
    * CPU and Memory Resources can be observed in
      [Google Cloud > Kubernetes Engine > Workloads][18]

#### 5. Report Results

* See [Distributed GCP Execution - Report Results](#4-report-results)

### Clean-up Environment

* See [Distributed GCP Execution - Clean-up Environment](#clean-up-environment)


[1]: https://locust.io/
[2]: https://miro.com/app/board/uXjVMx-kx9Q=/
[3]: https://docs.google.com/spreadsheets/d/1-nF8zR98IiLZlwP0ISwOB8cWRs74tJGM1PpgDSWdayE/edit#gid=0
[4]: https://drive.google.com/drive/folders/1LgucOMp1E1ptOyK19ohqiGeCMJmgf0px
[5]: https://python-poetry.org/docs/#installation
[6]: https://github.com/pyenv/pyenv#installation
[7]: https://github.com/pyenv/pyenv-virtualenv#installation
[8]: https://pycqa.github.io/isort/
[9]: https://black.readthedocs.io/en/stable/
[10]: https://flake8.pycqa.org/en/latest/
[11]: https://mypy-lang.org/
[12]: https://docs.locust.io/en/stable/configuration.html#environment-variables
[13]: https://docs.locust.io/en/stable/writing-a-locustfile.html#wait-time
[14]: https://docs.locust.io/en/stable/writing-a-locustfile.html#task-decorator
[15]: https://console.cloud.google.com/home/dashboard?q=search&referrer=search&project=spheric-keel-331521&cloudshell=false
[16]: https://console.cloud.google.com/artifacts/docker/spheric-keel-331521/us-central1/autopush?project=spheric-keel-331521
[17]: https://earthangel-b40313e5.influxcloud.net/d/do4mmwcVz/autopush-gcp?orgId=1&refresh=1m
[18]: https://console.cloud.google.com/kubernetes/list/overview?cloudshell=false&project=spheric-keel-331521
