# Docker build for Reliability Reporter.

FROM python:3.12-alpine

ADD . /app
WORKDIR /app
# should this be templated? Some have a path here I presume gets loaded via a Docker build.
# (e.g. fxa uses `/app/keys/keyfile_name.json`)
# loaded va the webservices-infra cronjob.yaml:: containers::volumeMounts
VOLUME [ "keys" "/app/keys" ]
ENV GOOGLE_APPLICATION_CREDENTIALS = /app/keys/service-account-key.json

# these will need to be templated in the calling k8s config.
# ENV AUTOEND__DB_SETTINGS = '{"table_name":"projects/test/instance/test/tables/autopush"}'
# ENV AUTOEND__DB_DSN = "grpc:://localhost:8086"
# ENV AUTOEND__RELIABILITY_DSN = "redis:://localhost"

# Generate the report
ENV AUTOTRACK_REPORT_BUCKET_NAME = "autopush_reliability"
ENV AUTOTRACK_OUTPUT="md json"
RUN pip install .
# Specifying the full paths here to avoid an issue with executables not found.
CMD ["/usr/local/bin/python3", "/app/reliability_report.py"]

