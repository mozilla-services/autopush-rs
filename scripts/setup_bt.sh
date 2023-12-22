#!/bin/sh -eu
cbt -project $1 -instance $2 createtable autopush
cbt -project $1 -instance $2 createfamily autopush message
cbt -project $1 -instance $2 createfamily autopush message_topic
cbt -project $1 -instance $2 createfamily autopush router
cbt -project $1 -instance $2 setgcpolicy autopush message maxage=1s
cbt -project $1 -instance $2 setgcpolicy autopush message_topic maxversions=1
cbt -project $1 -instance $2 setgcpolicy autopush router maxversions=1
