# Bridge system overview

Working with mobile devices can present many challenges. One very significant one
deals with how mobile devices save battery very aggressively. Using your
mobile devices CPU and radio both require considerable battery power. This means
that maintaining something like a constant connection to a remote server, or
regularly "pinging" a server can cause your device to wake, spin up the CPU and
use the radio to connect to local wifi or cellular networks. This may cause your
application to be quickly flagged by the operating system and either aggressively
deactivated, or be flagged for removal.

Fortunately, the major mobile OS providers offer a way to send messages to devices
on their networks. These systems operate similarly to the way Push works, but
have their own special considerations. In addition, we want to make sure that
messages remain encrypted while passing through these systems. The benefit of
using these sorts of systems is that message delivery is effectively "free",
and apps that use these systems are not flagged for removal.

Setting up the client portion of these systems is outside the scope of this
document, however the providers of these networks have great documentation that
can help get you started.

As a bit of shorthand, we refer to these proprietary mobile messaging systems as
"bridge" systems, since they act as a metaphorical bridge between our servers and
our applications.

How we connect and use these systems is described in the following documents:

* [Apple Push Notification service (APNs)](apns.md)

* [Google's Fire Cloud Messaging service (FCM)](fcm.md)