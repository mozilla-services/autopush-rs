# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at http://mozilla.org/MPL/2.0/.

# =============================================================================
# Pull in the version of cargo-chef we plan to use, so that all the below steps
# use a consistent set of versions.
# RUST_VER
FROM lukemathwalker/cargo-chef:0.1.77-rust-1.96-slim-bookworm AS chef
WORKDIR /app

# =============================================================================
# Analyze the project, and produce a plan to compile its dependcies. This will
# be run every time. The output should only change if the dependencies of the
# project change, or if significant details of the build process change.
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# =============================================================================
# Use the plan from above to build only the dependencies of the project. This
# should almost always be pulled straight from cache unless dependencies or the
# build process change.
FROM chef AS cacher
COPY --from=planner /app/recipe.json recipe.json
# Must match the `builder` stage's flags below: cook a different feature set and
# every fingerprint changes, so `builder` rebuilds the whole dependency tree
# instead of reusing what was cooked here. Keep the two ARGs in lockstep.
ARG CARGO_BUILD_ARGS="--features=emulator"

RUN \
  apt-get -qq update && \
  apt-get -qq install --no-install-recommends -y \
  cmake \
  clang \
  libssl-dev \
  ca-certificates \
  pkg-config \
  build-essential

RUN cargo chef cook --recipe-path recipe.json ${CARGO_BUILD_ARGS}

# =============================================================================
# Now build the project, taking advantage of the cached dependencies from above.
FROM cacher AS builder
# Must match the cacher's ARG above. Storage backends are mutually exclusive
# builds, so which one the integration suite exercises is decided here; defaults
# to Bigtable-against-the-emulator and `docker-compose-redis.yml` overrides it.
ARG CARGO_BUILD_ARGS="--features=emulator"

RUN mkdir -m 755 bin
RUN cargo --version && \
  rustc --version
COPY . .

RUN cargo build ${CARGO_BUILD_ARGS}

FROM python:3.12-slim-bookworm AS integration-tests

# =============================================================================
# Setup Integration test image

LABEL org.opencontainers.image.authors="contextual-services-team@mozilla.com"

ENV LANG=C.UTF-8
ENV PYTHONUNBUFFERED=1

ENV PATH=$PATH:/root/.cargo/bin
ENV PYTHON_VENV=/venv
ENV RUST_LOG="autopush=debug,autopush_common=debug,autoendpoint=debug,autoconnect=debug,slog_mozlog_json=info,warn"
ENV DB_DSN=grpc://localhost:8086

# Add gcc since there are no wheels for some packages for arm64/aarch64
# (g++/make for gevent on pypy)
RUN apt-get update && apt install -y --no-install-recommends \
  git \
  build-essential \
  curl \
  gpg

RUN python -m venv ${PYTHON_VENV}
ENV PATH="${PYTHON_VENV}/bin:${PATH}"

RUN python -m pip install --upgrade pip

# Setup poetry and install requirements
ENV POETRY_VIRTUALENVS_CREATE=false \
  POETRY_VERSION=1.7.0
RUN python -m pip install --no-cache-dir --quiet poetry
COPY ./tests/pyproject.toml ./tests/poetry.lock ./
RUN poetry install --only=integration --no-root --no-interaction --no-ansi

# Setup cloud big table. `cbt` is only used by `scripts/setup_bt.sh`; backends
# that don't need it (e.g. redis) can turn it off to save some build time
ARG INSTALL_CBT=1
RUN if [ "${INSTALL_CBT}" = "1" ]; then \
  curl https://packages.cloud.google.com/apt/doc/apt-key.gpg | gpg --dearmor -o /usr/share/keyrings/cloud.google.gpg && \
  echo "deb [signed-by=/usr/share/keyrings/cloud.google.gpg] https://packages.cloud.google.com/apt cloud-sdk main" | tee -a /etc/apt/sources.list.d/google-cloud-sdk.list && \
  apt-get update -y && apt install google-cloud-cli-cbt -y; \
  fi

COPY . /code

COPY --from=builder /app/target/debug /code/target/debug
COPY --from=builder /app/version.json /code

WORKDIR /code

RUN chmod +x scripts/setup_bt.sh

# Pytest markers are defined in the `pyproject.toml`:`[tool.pytest.ini_options]` file.
CMD ["sh", "-c", "./scripts/setup_bt.sh && poetry run pytest tests/integration/test_integration_all_rust.py --junit-xml=integration__results.xml -v"]
