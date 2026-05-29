SHELL := /bin/sh
CARGO = cargo
# For unknown reasons, poetry on CI will sometimes "forget" what it's current path is, which
# can confuse relative path lookups.
# Let's be very explicit about it for now.
TESTS_DIR := $(shell pwd)/tests
TEST_RESULTS_DIR ?= workspace/test-results

# In order to be consumed by the ETE Test Metric Pipeline, files need to follow a strict naming convention:
# {job_number}__{utc_epoch_datetime}__{repository}__{workflow}__{test_suite}__results{-index}.xml
WORKFLOW := build-test-deploy
EPOCH_TIME := $(shell date +"%s")
TEST_FILE_PREFIX := $(if $(CIRCLECI),$(CIRCLE_BUILD_NUM)__$(EPOCH_TIME)__$(CIRCLE_PROJECT_REPONAME)__$(WORKFLOW)__)
UNIT_JUNIT_XML := $(TEST_RESULTS_DIR)/$(TEST_FILE_PREFIX)unit__results.xml
UNIT_COVERAGE_JSON := $(TEST_RESULTS_DIR)/$(TEST_FILE_PREFIX)unit__coverage.json
INTEGRATION_JUNIT_XML := $(TEST_RESULTS_DIR)/$(TEST_FILE_PREFIX)integration__results.xml
INTEGRATION_JUNIT_XML_LEGACY := $(TEST_RESULTS_DIR)/$(TEST_FILE_PREFIX)integration__legacy-results.xml

# NOTE: Do not be clever.
# The integration tests (and a few others) use pytest markers to control
# the tests that are being run. These markers are set and defined within
# the `./pyproject.toml`. That is the single source of truth.
PYTEST_ARGS := ${PYTEST_ARGS}
INTEGRATION_TEST_DIR := $(TESTS_DIR)/integration
INTEGRATION_TEST_FILE := $(INTEGRATION_TEST_DIR)/test_integration_all_rust.py
NOTIFICATION_TEST_DIR := $(TESTS_DIR)/notification
LOAD_TEST_DIR := $(TESTS_DIR)/load
POETRY := poetry --directory $(TESTS_DIR)
DOCKER_COMPOSE := docker compose
PYPROJECT_TOML := $(TESTS_DIR)/pyproject.toml
POETRY_LOCK := $(TESTS_DIR)/poetry.lock
FLAKE8_CONFIG := $(TESTS_DIR)/.flake8
LOCUST_HOST := "wss://autoconnect.stage.mozaws.net"
INSTALL_STAMP := .install.stamp


## Do an SOP build.
PHONY: build
build:
	python3 scripts/prefix_build_flags.py cargo build --features=production

## Build the stand-alone/enterprise version of Autopush.
PHONY: build-enterprise
build-enterprise:
	python3 scripts/prefix_build_flags.py cargo build --features=enterprise

## Some systems may require a dedicated environment to build Autopush. An example of this
## was a problem with versions of Debian Trixie having changed a header file, which caused
## grpcio to fail to build. The `Dockerfile-dev` environment allows you to create a container
## built off of a prior Debian version which will allow you to build and test Autopush. (It's
## worth noting that the built image runs fine on Trixie.)
.PHONY: docker-dev-build
docker-dev-build:
	echo $(CMAKE_POLICY_VERSION_MINIMUM)
	docker build -f Dockerfile-dev -t autopush-dev .

## Initialize the docker environment. 
## This is actually pretty useful to initialize a local build environment as well.
.PHONY: docker-init
docker-init:
	sudo apt update
	sudo apt-get install build-essential libffi-dev libssl-dev pypy3-dev python3-virtualenv python3-poetry python-is-python3 git glibc-source cmake clang --assume-yes
	cargo install cargo-audit
	rustup update 1.93.1 	## RUST_VER

## Install python poetry dependencies.
.PHONY: install
install: $(INSTALL_STAMP)  ##  Install dependencies with poetry
$(INSTALL_STAMP): $(PYPROJECT_TOML) $(POETRY_LOCK)
	echo "Installing poetry dependencies. Autopush generally doesn't have an 'install', since it"
	echo "runs as a service and the docker image points to the binary directly. If you want to "
	echo "run the applications you can either `"
	@if [ -z $(POETRY) ]; then echo "Poetry could not be found. See https://python-poetry.org/docs/"; exit 2; fi
	$(POETRY) install
	touch $(INSTALL_STAMP)

install_poetry:
	curl -sSL https://install.python-poetry.org | python3 - --version 2.0.0

## Upgrade rust crate dependencies. This doesn't bump everything, but it can help
## with `cargo audit` issues.
upgrade:
	$(CARGO) install cargo-edit ||
		echo "\n$(CARGO) install cargo-edit failed, continuing.."
	$(CARGO) upgrade
	$(CARGO) update

## Run the unit tests.
.ONESHELL:
unit-test:
	cargo llvm-cov --summary-only --json --output-path $(UNIT_COVERAGE_JSON) \
	  nextest --features=emulator --features=bigtable --jobs=2 --profile=ci; exit_code=$$?
	mv target/nextest/ci/junit.xml $(UNIT_JUNIT_XML)
	exit $$exit_code

## Build the integration test docker image.
build-integration-test:
	$(DOCKER_COMPOSE) -f $(INTEGRATION_TEST_DIR)/docker-compose.yml build

## Run the integration tests inside of the integration test docker container.
## If you want to run these tests locally, use `make integration-test-local` instead.
.ONESHELL:
integration-test:
	$(DOCKER_COMPOSE) -f $(INTEGRATION_TEST_DIR)/docker-compose.yml run -it --name integration-tests tests; exit_code=$$?
	docker cp integration-tests:/code/integration__results.xml $(INTEGRATION_JUNIT_XML)
	exit $$exit_code

## Clean up the integration test toys.
integration-test-clean:
	$(DOCKER_COMPOSE) -f $(INTEGRATION_TEST_DIR)/docker-compose.yml down
	docker rm integration-tests

## This runs the older integration tests. Chances are good you'll not need these.
integration-test-legacy: ## pytest markers are stored in `tests/pytest.ini`
	$(POETRY) -V
	$(POETRY) install --without dev,load,notification --no-root
	$(POETRY) run pytest $(INTEGRATION_TEST_FILE) \
		--junit-xml=$(INTEGRATION_JUNIT_XML_LEGACY) \
		-v $(PYTEST_ARGS)

## Run the integration tests locally. Useful for debugging and doing isolate tests.
## (remember, you can pass a partial test name to run matching tests, e.g. 
## `PYTEST_ARGS="-k test_fcm" make integration-test-local`)
integration-test-local: ## pytest markers are stored in `tests/pytest.ini`
	$(POETRY) -V
	$(POETRY) install --without dev,load,notification --no-root
	$(POETRY) run pytest $(INTEGRATION_TEST_FILE) \
		--junit-xml=$(INTEGRATION_JUNIT_XML) \
		-v $(PYTEST_ARGS)

## Run the notification tests
notification-test:
	$(DOCKER_COMPOSE) -f $(NOTIFICATION_TEST_DIR)/docker-compose.yml build
	$(DOCKER_COMPOSE) -f $(NOTIFICATION_TEST_DIR)/docker-compose.yml up -d server
	$(DOCKER_COMPOSE) -f $(NOTIFICATION_TEST_DIR)/docker-compose.yml run -e NOTIFICATION_TEST_ENV=$(NOTIFICATION_TEST_ENV) --remove-orphans -it --name notification-tests tests
	docker cp notification-tests:/code/notification-tests.xml $(NOTIFICATION_TEST_DIR)

notification-test-clean:
	docker rm notification-tests

## Build the applications with profiling information active (see Cargo.toml for details).
# `prefix_build_flags.py` is a hack to work around the fact that we need to set some platform specific env vars for the build, 
# but doing so in a Makefile is a nightmare. This way, we can just call this script with the same args we would 
# normally call cargo with, and it will add the necessary env vars before running the command.
.PHONY: build-profile
build-profile:
	python3 scripts/prefix_build_flags.py RUSTFLAGS="-C force-frame-pointers=yes" cargo build --profile profile

## Python Hygiene functions
.PHONY: format
format: $(INSTALL_STAMP)  ##  Sort imports and reformats code
	$(POETRY) run isort $(TESTS_DIR)
	$(POETRY) run black $(TESTS_DIR)

.PHONY: isort
isort: $(INSTALL_STAMP)  ##  Run isort
	$(POETRY) run isort --check-only $(TESTS_DIR)

.PHONY: black
black: $(INSTALL_STAMP)  ##  Run black
	$(POETRY) run black --quiet --diff --check $(TESTS_DIR)

.PHONY: flake8
flake8: $(INSTALL_STAMP)  ##  Run flake8
	$(POETRY) run flake8 --config $(FLAKE8_CONFIG) $(TESTS_DIR)

.PHONY: bandit
bandit: $(INSTALL_STAMP)  ##  Run bandit
	$(POETRY) run bandit --quiet -r -c $(PYPROJECT_TOML) $(TESTS_DIR)

.PHONY: mypy
mypy: $(INSTALL_STAMP)  ##  Run mypy
	$(POETRY) run mypy --config-file=$(PYPROJECT_TOML) $(TESTS_DIR)

.PHONY: pydocstyle
pydocstyle: $(INSTALL_STAMP)  ##  Run pydocstyle
	$(POETRY) run pydocstyle -es --count --config=$(PYPROJECT_TOML) $(TESTS_DIR)

lint:
	$(POETRY) -V
	$(POETRY) install
	$(POETRY) run isort --sp $(PYPROJECT_TOML) -c $(TESTS_DIR)
	$(POETRY) run black --quiet --diff --config $(PYPROJECT_TOML) --check $(TESTS_DIR)
	$(POETRY) run flake8 --config $(FLAKE8_CONFIG) $(TESTS_DIR)
	$(POETRY) run bandit --quiet -r -c $(PYPROJECT_TOML) $(TESTS_DIR)
	$(POETRY) run pydocstyle --config=$(PYPROJECT_TOML) $(TESTS_DIR)
	$(POETRY) run mypy $(TESTS_DIR) --config-file=$(PYPROJECT_TOML)

## Load Testing
load:
	LOCUST_HOST=$(LOCUST_HOST) \
	  $(DOCKER_COMPOSE) \
      -f $(LOAD_TEST_DIR)/docker-compose.yml \
      -p autopush-rs-load-tests \
      up --scale locust_worker=1

load-clean:
	$(DOCKER_COMPOSE) \
      -f $(LOAD_TEST_DIR)/docker-compose.yml \
      -p autopush-rs-load-tests \
      down
	docker rmi locust

## Generate the documentation. (This is also kinda hacky, but works)
.PHONY: doc-prev
doc-prev:  ##  Generate live preview of autopush docs via browser
	mdbook clean docs/
	mdbook build docs/
	mdbook serve docs/ --open

##  Generate the CryptoKey values
.PHONY: gen-key
gen-key:   
	$(POETRY) install --no-root
	$(POETRY) run python3 $(CURDIR)/scripts/autokey.py
