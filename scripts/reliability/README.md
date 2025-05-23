# Push Reliability Support Cron

This image runs the Push Reliability Support script. This is used by the optional Push Reliability system.

Push Reliability is determined by examining the flow of messages that are generated and consumed by Mozilla, specifically the "Send Tab" control messages.
Messages are tracked by when they reach significant points in the code (milestones). See the `autopush_common::reliability::ReliabilityState` for individual milestones.

During the processing, however, there are two functions that should occur to aid in getting a clear picture of the states. The first step is to generate a daily report of the milestones. This script generates this report using the Jinja2 templates listed in `./templates`, as well as a JSON output designed to be machine processed. The other is a bit of house-keeping to ensure that milestones that have reached "terminal" (states where they will not transition to a different state later), are regularly removed. This will prevent out-dated data from collecting and producing less useful reports.

This task is designed to be run as a Kubernetes Cron Job. The actual configuration for that is contained in a different, internally used repo, but all that is doing is setting a few general values (See the commented values in the `Dockerfile`.) These can optionally be defined in a `reliability.toml` configuration file. In addition the Docker image should be run with appropriate keys to the GCP data that we require for Key/Value & Bigtable access.

There are a few additional config options included here to allow for future flexibility, including the `app.yaml` file, which is used by some GCP task execution functions. These may be removed later.
