SHELL := /bin/sh
CARGO = cargo
TESTS_DIR := tests
TEST_RESULTS_DIR ?= workspace/test-results
# NOTE: Do not be clever.
# The integration tests (and a few others) use pytest markers to control
# the tests that are being run. These markers are set and defined within
# the `.tests/pytest.ini`. That is the single source of truth.
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


.PHONY: install
install: $(INSTALL_STAMP)  ##  Install dependencies with poetry
$(INSTALL_STAMP): $(PYPROJECT_TOML) $(POETRY_LOCK)
	@if [ -z $(POETRY) ]; then echo "Poetry could not be found. See https://python-poetry.org/docs/"; exit 2; fi
	$(POETRY) install
	touch $(INSTALL_STAMP)

upgrade:
	$(CARGO) install cargo-edit ||
		echo "\n$(CARGO) install cargo-edit failed, continuing.."
	$(CARGO) upgrade
	$(CARGO) update

build-integration-test:
	$(DOCKER_COMPOSE) -f $(INTEGRATION_TEST_DIR)/docker-compose.yml build

integration-test:
	$(DOCKER_COMPOSE) -f $(INTEGRATION_TEST_DIR)/docker-compose.yml run -it --name integration-tests tests
	docker cp integration-tests:/code/integration_test_results.xml $(INTEGRATION_TEST_DIR)

integration-test-clean:
	$(DOCKER_COMPOSE) -f $(INTEGRATION_TEST_DIR)/docker-compose.yml down
	docker rm integration-tests

integration-test-legacy: ## pytest markers are stored in `tests/pytest.ini`
	$(POETRY) -V
	$(POETRY) install --without dev,load,notification --no-root
	$(POETRY) run pytest $(INTEGRATION_TEST_FILE) \
		--junit-xml=$(TEST_RESULTS_DIR)/integration_test_legacy_results.xml \
		-v $(PYTEST_ARGS)

integration-test-local: ## pytest markers are stored in `tests/pytest.ini`
	$(POETRY) -V
	$(POETRY) install --without dev,load,notification --no-root
	$(POETRY) run pytest $(INTEGRATION_TEST_FILE) \
		--junit-xml=$(TEST_RESULTS_DIR)/integration_test_results.xml \
		-v $(PYTEST_ARGS)

notification-test:
	$(DOCKER_COMPOSE) -f $(NOTIFICATION_TEST_DIR)/docker-compose.yml build
	$(DOCKER_COMPOSE) -f $(NOTIFICATION_TEST_DIR)/docker-compose.yml up -d server
	$(DOCKER_COMPOSE) -f $(NOTIFICATION_TEST_DIR)/docker-compose.yml run -e NOTIFICATION_TEST_ENV=$(NOTIFICATION_TEST_ENV) --remove-orphans -it --name notification-tests tests
	docker cp notification-tests:/code/notification-tests.xml $(NOTIFICATION_TEST_DIR)

notification-test-clean:
	docker rm notification-tests

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

.PHONY: doc-prev
doc-prev:  ##  Generate live preview of autopush docs via browser
	mdbook clean docs/
	mdbook build docs/
	mdbook serve docs/ --open
