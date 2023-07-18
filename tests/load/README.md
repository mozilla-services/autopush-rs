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

Contributors to this project are expected to execute the following tools for import 
sorting, linting, style guide enforcement and static type checking.
Configurations are set in the `pyproject.toml` and `.flake8` files.

**[isort][4]**
 ```shell
poetry run isort locustfile.py
 ```

**[black][5]**
 ```shell
poetry run black locustfile.py
 ```

**[flake8][6]**
 ```shell
poetry run flake8 locustfile.py
 ```


## Local Execution

There are 3 docker-compose files pertaining to the different environments autopush can run on:
`docker-compose.prod.yml`
`docker-compose.stage.yml`
`docker-compose.dev.yml`

You can select which environment you want to run against by choosing the appropriate file. The environment will be setup for you with the correct URLs.

Ex:
```shell
docker-compose -f docker-compose.stage.yml up --build --scale locust_worker=1
```

This will run build and start the locust session for the Stage environment.


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

Execute the following from the `load` directory:
```shell
docker-compose -f docker-compose.stage.yml down
docker rmi locust
```

[1]: https://python-poetry.org/docs/#installation
[2]: https://github.com/pyenv/pyenv#installation
[3]: https://github.com/pyenv/pyenv-virtualenv#installation
[4]: https://pycqa.github.io/isort/
[5]: https://black.readthedocs.io/en/stable/
[6]: https://flake8.pycqa.org/en/latest/