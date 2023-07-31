# Autopush Load (Locust) Tests

This directory contains the automated load test suite for autopush. These tests are run using a tool named [locust](https://locust.io/).

## Contributing

This project uses [Poetry][1] for dependency management. For environment setup it is 
recommended to use [pyenv][2] and [pyenv-virtualenv][3], as they work nicely with 
Poetry.

Project dependencies are listed in the `pyproject.toml` file.
To install the dependencies execute:
```shell
poetry install
```

Contributors to this project are expected to execute isort, black, flake8 and mypy for 
import sorting, linting, style guide enforcement and static type checking respectively.
Configurations are set in the `pyproject.toml` and `.flake8` files.

The tools can be executed with the following command from the root directory:

```shell
make lint
```

## Local Execution

Follow the steps bellow to execute the load tests locally:

### Setup Environment

#### 1. Configure Environment Variables

Environment variables, listed bellow or specified by [Locust][8], can be set in
`tests\load\docker-compose.yml`.

| Environment Variable | Node(s)          | Description                     |
|----------------------|------------------|---------------------------------|
| SERVER_URL           | master & worker  | The autopush web socket address |
| ENDPOINT_URL         | master & worker  | The autopush HTTP address       |

#### 2. Host Locust via Docker

Execute the following from the root directory:

```shell
make load
```

### Run Test Session

#### 1. Start Load Test

* In a browser navigate to `http://localhost:8089/`
* Select "Start Swarming"

#### 2. Stop Load Test

Select the 'Stop' button in the top right hand corner of the Locust UI, after the 
desired test duration has elapsed. If the 'Run time' or 'Duration' is set in step 1, 
the load test will stop automatically.

#### 3. Analyse Results

* Only client-side measures, provided by Locust, are available for local execution

### Clean-up Environment

#### 1. Remove Load Test Docker Containers

Execute the following from the root directory:

```shell
make load-clean
```

[1]: https://python-poetry.org/docs/#installation
[2]: https://github.com/pyenv/pyenv#installation
[3]: https://github.com/pyenv/pyenv-virtualenv#installation
[4]: https://pycqa.github.io/isort/
[5]: https://black.readthedocs.io/en/stable/
[6]: https://flake8.pycqa.org/en/latest/
[7]: https://mypy-lang.org/
[8]: https://docs.locust.io/en/stable/configuration.html#environment-variables