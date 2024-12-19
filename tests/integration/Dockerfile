# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at http://mozilla.org/MPL/2.0/.

FROM python:3.12-slim-bookworm

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
  gpg \
  build-essential \
  python3-dev \
  curl \
  libstdc++6 \
  libstdc++-12-dev \
  libssl-dev \
  pkg-config \
  cmake

RUN python -m venv ${PYTHON_VENV}
ENV PATH="${PYTHON_VENV}/bin:${PATH}"

RUN python -m pip install --upgrade pip

# Install Rust
RUN curl https://sh.rustup.rs -sSf | sh -s -- --default-toolchain 1.81 -y
RUN rustc --version

# Setup poetry and install requirements
ENV POETRY_VIRTUALENVS_CREATE=false \
  POETRY_VERSION=1.7.0
RUN python -m pip install --no-cache-dir --quiet poetry
COPY ./tests/pyproject.toml ./tests/poetry.lock ./tests/pytest.ini ./
RUN poetry install --only=integration --no-interaction --no-ansi

# Setup cloud big table
RUN curl https://packages.cloud.google.com/apt/doc/apt-key.gpg | gpg --dearmor -o /usr/share/keyrings/cloud.google.gpg
RUN echo "deb [signed-by=/usr/share/keyrings/cloud.google.gpg] https://packages.cloud.google.com/apt cloud-sdk main" | tee -a /etc/apt/sources.list.d/google-cloud-sdk.list
RUN apt-get update -y && apt install google-cloud-cli-cbt -y

COPY . /code

WORKDIR /code

# Build app
RUN cargo build --features=emulator

RUN chmod +x scripts/setup_bt.sh

# Pytest markers are defined in the `pytest.ini` file.
CMD ["sh", "-c", "./scripts/setup_bt.sh && poetry run pytest tests/integration/test_integration_all_rust.py --config-file=pytest.ini --junit-xml=integration_test_results.xml -v"]
