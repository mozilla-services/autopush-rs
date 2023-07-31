SHELL := /bin/sh
CARGO = cargo
LOAD_TEST_DIR := tests/load
POETRY := poetry --directory $(LOAD_TEST_DIR)
PYPROJECT_TOML := $(LOAD_TEST_DIR)/pyproject.toml
FLAKE8_CONFIG := $(LOAD_TEST_DIR)/.flake8
STAGE_SERVER_URL := "wss://autopush.stage.mozaws.net"
STAGE_ENDPOINT_URL := "https://updates-autopush.stage.mozaws.net"

.PHONY: ddb

ddb:
	mkdir $@
	curl -sSL http://dynamodb-local.s3-website-us-west-2.amazonaws.com/dynamodb_local_latest.tar.gz | tar xzvC $@

upgrade:
	$(CARGO) install cargo-edit ||
		echo "\n$(CARGO) install cargo-edit failed, continuing.."
	$(CARGO) upgrade
	$(CARGO) update

lint:
	$(POETRY) -V
	$(POETRY) install
	$(POETRY) run isort --sp $(PYPROJECT_TOML) -c $(LOAD_TEST_DIR)
	$(POETRY) run black --quiet --diff --config $(PYPROJECT_TOML) --check $(LOAD_TEST_DIR)
	$(POETRY) run flake8 --config $(FLAKE8_CONFIG) $(LOAD_TEST_DIR)
	$(POETRY) run mypy $(LOAD_TEST_DIR) --config-file=$(PYPROJECT_TOML)

load:
	SERVER_URL=$(STAGE_SERVER_URL) ENDPOINT_URL=$(STAGE_ENDPOINT_URL) \
	  docker-compose \
      -f $(LOAD_TEST_DIR)/docker-compose.yml \
      -p autopush-rs-load-tests \
      up --scale locust_worker=1

load-clean:
	docker-compose \
      -f $(LOAD_TEST_DIR)/docker-compose.yml \
      -p autopush-rs-load-tests \
      down
	docker rmi locust
