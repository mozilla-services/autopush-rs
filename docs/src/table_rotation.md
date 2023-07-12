# DynamoDB Message Table Rotation (legacy)

As of version 1.45.0, message table rotation can be disabled. This is
because DynamoDB now provides automatic entry expiration. This is
controlled in our data by the "expiry" field. (_**Note**_, field
expiration is only available in full DynamoDB, and is not replicated
with the mock DynamoDB API provided for development.) The following
feature is disabled with the `no_table_rotation` flag set in the
`autopush_shared.ini` configuration file.

If table rotation is disabled, the last message table used will become
'frozen' and will be used for all future messages. While this may not be
aesthetically pleasing, it's more efficient than copying data to a new,
generic table. If it's preferred, service can be shut down, previous
tables dropped, the current table renamed, and service brought up again.

**Message Table Rotation information**

To avoid costly table scans, autopush used a rotating message and
router table. Clients that hadn't connected in 30-60 days would have
their router and message table entries dropped and needed to
re-register.
Tables were post-fixed with the year/month they were meant for, i.e. :
    messages_2015_02
Tables must have been created and had their read/write units properly
allocated by a separate process in advance of the month switch-over as
autopush nodes would assume the tables already existed. Scripts [were
provided\(https://github.com/mozilla-services/autopush/blob/master/maintenance.py)
that could be run weekly to ensure all necessary tables were
present, and tables old enough were dropped.

Within a few days of the new month, the load on the prior months table
would fall as clients transition to the new table. The read/write units
on the prior month may then be lowered.

## DynamoDB Rotating Message Table Interaction Rules (legacy)

Due to the complexity of having notifications spread across two tables,
several rules are used to avoid losing messages during the month
transition.

The logic for connection nodes is more complex, since only the
connection node knows when the client connects, and how many messages it
has read through.

When table rotation is allowed, the router table uses the `curmonth`
field to indicate the last month the client has read notifications
through. This is independent of the last_connect since it is possible
for a client to connect, fail to read its notifications, then reconnect.
This field is updated for a new month when the client connects **after**
it has ack'd all the notifications out of the last month.

To avoid issues with time synchronization, the node the client is
connected to acts as the source of truth for when the month has flipped
over. Clients are only moved to the new table on connect, and only after
reading/acking all the notifications for the prior month.

#### Rules for Endpoints

1. Check the router table to see the current_month the client is on.

2. Read the chan list entry from the appropriate month message table to
    see if its a valid channel.

    If its valid, move to step 3.

3. Store the notification in the current months table if valid. (_**Note**_
    that this step does not copy the blank entry of valid channels)

#### Rules for Connection Nodes

After Identification:

1. Check to see if the current_month matches the current month, if it
    does then proceed normally using the current months message table.

    If the connection node month does not match stored current_month in
    the clients router table entry, proceed to step 2.

2. Read notifications from prior month and send to client.

    Once all ACKs are received for all the notifications for that month
    proceed to step 3.

3. Copy the blank message entry of valid channels to the new month
    message table.

4. Update the router table for the current_month.

During switchover, only after the router table update are new commands
from the client accepted.

Handling of Edge Cases:

* Connection node gets more notifications during step 3, enough to
    buffer, such that the endpoint starts storing them in the previous
    current_month. In this case the connection node will check the old
    table, then the new table to ensure it doesn't lose message during
    the switch.
* Connection node dies, or client disconnects during step 3/4. Not a
    problem as the reconnect will pick it up at the right spot.
