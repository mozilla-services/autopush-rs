FROM python:3-slim

ENV POETRY_HOME="/opt/poetry" \
    POETRY_VIRTUALENVS_IN_PROJECT=1 \
    POETRY_NO_INTERACTION=1 \
    GECKODRIVER="0.35.0"

ENV PATH="$POETRY_HOME/bin:$PATH"

ENV NOTIFICATION_TEST_ENV ="dev"

RUN apt-get update && apt-get install -y --no-install-recommends \
    gcc \
    musl-dev \
    xvfb \
    xauth \
    curl \
    wget \
    gnupg2 \
    xz-utils \
    libgtk-3-0 \
    libdbus-glib-1-2 \
    libxt6 \
    libx11-xcb1 \
    libxcomposite1 \
    libxdamage1 \
    libxfixes3 \
    libxrender1 \
    libxext6 \
    libxrandr2 \
    libasound2 \
    libpango-1.0-0 \
    libpangocairo-1.0-0 \
    libdrm2 \
    libgbm1 \
    libatspi2.0-0 \
    libglib2.0-0 \
    libnss3 \
    libfontconfig1 \
    libdbus-glib-1-2 \
    && apt-get clean -y

# Download and install the latest Firefox release
RUN wget -O - "https://download.mozilla.org/?product=firefox-latest&os=linux64&lang=en-US" | tar -xJ -C /opt/ \
    && ln -s /opt/firefox/firefox /usr/local/bin/firefox

# Install GeckoDriver
RUN wget https://github.com/mozilla/geckodriver/releases/download/v${GECKODRIVER}/geckodriver-v${GECKODRIVER}-linux64.tar.gz \
    && tar -xvzf geckodriver-v${GECKODRIVER}-linux64.tar.gz \
    && mv geckodriver /usr/local/bin/ \
    && rm geckodriver-v${GECKODRIVER}-linux64.tar.gz

RUN curl -sSL https://install.python-poetry.org | python3 -

WORKDIR /code
ADD ./tests/notification/ /code
ADD ./tests/poetry.lock /code
ADD ./tests/pyproject.toml /code

RUN poetry install --no-root --only=notification

CMD xvfb-run --auto-servernum --server-args="-screen 0 1920x1080x24" poetry run pytest --driver Firefox --env ${NOTIFICATION_TEST_ENV}
