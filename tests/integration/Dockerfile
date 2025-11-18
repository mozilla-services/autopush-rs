# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at http://mozilla.org/MPL/2.0/.

# =============================================================================
# Pull in the version of cargo-chef we plan to use, so that all the below steps
# use a consistent set of versions.
# RUST_VER
FROM lukemathwalker/cargo-chef:0.1.73-rust-1.90-slim-bookworm AS chef
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

RUN \
  apt-get -qq update && \
  apt-get -qq install --no-install-recommends -y \
  cmake \
  clang \
  libssl-dev \
  ca-certificates \
  pkg-config \
  build-essential

RUN cargo chef cook --recipe-path recipe.json

# =============================================================================
# Now build the project, taking advantage of the cached dependencies from above.
FROM chef AS builder
ARG APT_CACHE_BUST

RUN mkdir -m 755 bin
RUN apt-get -qq update && \
  apt-get -qq upgrade && apt-get -qq install --no-install-recommends -y \
  cmake \
  clang \
  libssl-dev \
  ca-certificates \
  libstdc++6 \
  libstdc++-12-dev

RUN cargo --version && \
  rustc --version
COPY . .
COPY --from=cacher /app/target target
COPY --from=cacher $CARGO_HOME $CARGO_HOME

RUN cargo build --features=emulator

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
RUN poetry install --only=integration --no-interaction --no-ansi

# Setup cloud big table
RUN curl https://packages.cloud.google.com/apt/doc/apt-key.gpg | gpg --dearmor -o /usr/share/keyrings/cloud.google.gpg
RUN echo "deb [signed-by=/usr/share/keyrings/cloud.google.gpg] https://packages.cloud.google.com/apt cloud-sdk main" | tee -a /etc/apt/sources.list.d/google-cloud-sdk.list
RUN apt-get update -y && apt install google-cloud-cli-cbt -y

COPY . /code

COPY --from=builder /app/target/debug /code/target/debug
COPY --from=builder /app/version.json /code

WORKDIR /code

RUN chmod +x scripts/setup_bt.sh

# Pytest markers are defined in the `pyproject.toml`:`[tool.pytest.ini_options]` file.
CMD ["sh", "-c", "./scripts/setup_bt.sh && poetry run pytest tests/integration/test_integration_all_rust.py --junit-xml=integration__results.xml -v"]
