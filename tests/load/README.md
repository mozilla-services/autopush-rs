# Autopush Load (Locust) Tests

This directory contains the automated load test suite for autopush. These tests are run
using a tool named [locust][1].

## Related Documentation

* [Autopush Load Test Class Diagram][2]
* [Autopush Load Test Spreadsheet][3]

## Contributing

This project uses [Poetry][4] for dependency management. For environment setup it is
recommended to use [pyenv][5] and [pyenv-virtualenv][6], as they work nicely with
Poetry.

Project dependencies are listed in the `pyproject.toml` file.
To install the dependencies execute:

```shell
poetry install
```

Contributors to this project are expected to execute [isort][7], [black][8], [flake8][9]
and [mypy][10] for import sorting, linting, style guide enforcement and static type
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

Environment variables, listed bellow or specified by [Locust][11], can be set in
`tests\load\docker-compose.yml`.

| Environment Variable | Node(s)         | Description                          |
|----------------------|-----------------|--------------------------------------|
| SERVER_URL           | master & worker | The autopush web socket address      |
| ENDPOINT_URL         | master & worker | The autopush HTTP address            |
| AUTOPUSH_WAIT_TIME   | master & worker | The wait time between task execution |

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
    * Spawn rate: 1
    * Host: 'https://updates-autopush.stage.mozaws.net'
    * Duration (Optional): 10m
    * Websocket URL: 'wss://autopush.stage.mozaws.net'
    * Endpoint URL: 'https://updates-autopush.stage.mozaws.net'
* Select "Start Swarming"

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

The load tests can be executed from the [contextual-services-test-eng cloud shell][12].

#### 2. Configure the Bash Script

* The `setup_k8s.sh` file, located in the `tests\load` directory, contains
  shell commands to **create** a GKE cluster, **setup** an existing GKE cluster or
  **delete** a GKE cluster
    * Execute the following from the `load` directory, to make the file executable:
      ```shell
      chmod +x setup_k8s.sh
      ```

#### 3. Create the GCP Cluster

* Execute the `setup_k8s.sh` file and select the **create** option, in order to
  initiate the process of creating a cluster, setting up the env variables and
  building the docker image
  ```shell
  ./setup_k8s.sh
  ```
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
      deploying them (see [Container Registry][13])

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
    * Host: 'https://updates-autopush.stage.mozaws.net'
    * Websocket URL: 'wss://autopush.stage.mozaws.net'
    * Endpoint URL: 'https://updates-autopush.stage.mozaws.net'
* Select "Start Swarming"

#### 2. Stop Load Test

Select the 'Stop' button in the top right hand corner of the Locust UI, after the
desired test duration has elapsed. If the 'Run time' or 'Duration' is set in step 1,
the load test will stop automatically.

#### 3. Analyse Results

**Failures**

* The number of responses with errors (non-200 response codes) should be `0`
* Locust reports Failures via the "autopush_failures.csv" file and the UI
  (under the "Failures" tab or the "Charts" tab)
* [Grafana][14] reports Failures via the "HTTP Response codes" chart and the
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
    * Download the results via the Locust UI or via command:
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
    * Upload the files to the [ConServ][15] drive and record the links in the
      spreadsheet

### Clean-up Environment

#### 1. Delete the GCP Cluster

Execute the `setup_k8s.sh` file and select the **delete** option

```shell
./tests/load/setup_k8s.sh
```

[1]: https://locust.io/
[2]: https://miro.com/app/board/uXjVMx-kx9Q=/
[3]: https://docs.google.com/spreadsheets/d/1-nF8zR98IiLZlwP0ISwOB8cWRs74tJGM1PpgDSWdayE/edit#gid=0
[4]: https://python-poetry.org/docs/#installation
[5]: https://github.com/pyenv/pyenv#installation
[6]: https://github.com/pyenv/pyenv-virtualenv#installation
[7]: https://pycqa.github.io/isort/
[8]: https://black.readthedocs.io/en/stable/
[9]: https://flake8.pycqa.org/en/latest/
[10]: https://mypy-lang.org/
[11]: https://docs.locust.io/en/stable/configuration.html#environment-variables
[12]: https://console.cloud.google.com/home/dashboard?q=search&referrer=search&project=spheric-keel-331521&cloudshell=false
[13]: https://console.cloud.google.com/compute/instances?project=spheric-keel-331521
[14]: https://earthangel-b40313e5.influxcloud.net/d/do4mmwcVz/autopush-gcp?orgId=1&refresh=1m
[15]: https://drive.google.com/file/d/1dETF1LhW3Msw54cp7iJ4pMPeZWEv3sUg/view?usp=drive_link
