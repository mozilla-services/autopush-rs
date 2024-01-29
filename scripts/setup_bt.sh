#!/bin/sh -eu
# Usage: setup_bt.sh [PROJECT] [INSTANCE]
#
# Arguments:
#   [PROJECT]  (default: test)
#   [INSTANCE] (default: test)

PROJECT=${1:-"test"}
INSTANCE=${2:-"test"}

TABLE_NAME=${TABLE_NAME:-"autopush"}
MESSAGE_FAMILY=${MESSAGE_FAMILY:-"message"}
MESSAGE_TOPIC_FAMILY=${MESSAGE_TOPIC_FAMILY:-"message_topic"}
ROUTER_FAMILY=${ROUTER_FAMILY:-"router"}

cbt -project $PROJECT -instance $INSTANCE createtable $TABLE_NAME
cbt -project $PROJECT -instance $INSTANCE createfamily $TABLE_NAME $MESSAGE_FAMILY
cbt -project $PROJECT -instance $INSTANCE createfamily $TABLE_NAME $MESSAGE_TOPIC_FAMILY
cbt -project $PROJECT -instance $INSTANCE createfamily $TABLE_NAME $ROUTER_FAMILY
cbt -project $PROJECT -instance $INSTANCE setgcpolicy $TABLE_NAME $MESSAGE_FAMILY "maxage=1s or maxversions=1"
cbt -project $PROJECT -instance $INSTANCE setgcpolicy $TABLE_NAME $MESSAGE_TOPIC_FAMILY "maxage=1s or maxversions=1"
cbt -project $PROJECT -instance $INSTANCE setgcpolicy $TABLE_NAME $ROUTER_FAMILY maxversions=1
