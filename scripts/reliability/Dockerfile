# Docker build for Reliability Reporter.

FROM python:3.12-alpine

ADD . /app
WORKDIR /app
# should this be templatized? Some have a path here I presume gets loaded via a Docker build.
# (e.g. fxa uses `/app/keys/keyfile_name.json`)
# loaded va cronjob.yaml:: containers::volumeMounts
ENV GOOGLE_APPLICATION_CREDENTIALS = /app/keys/service-account-key.json
# these will need to be templatized
ENV AUTOEND__DB_SETTINGS = '{"table_name":"projects/test/instance/test/tables/autopush"}'
ENV AUTOEND__DB_DSN = "grpc:://localhost:8086"
ENV AUTOEND__RELIABILITY_DSN = "redis:://localhost"
# Generate the report
ENV AUTOTRACK_REPORT_BUCKET_NAME = "autopush_reliability"
ENV AUTOTRACK_OUTPUT="md json raw"
RUN pip install .
CMD ["python3", "reliability_report.py"]

