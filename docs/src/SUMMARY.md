# Summary
<!-- NOTE: `mdbook build` will create documents if they're not present. It uses
     the path specified in the parenthesis. It has no idea about internal links
     so (foo.md#bar) will create a doc named "foo.md#bar".
-->
* [Introduction](index.md)
* [Developer Onboarding](onboarding/index.md)
  * [Actors, services, and registration](onboarding/actors-and-services.md)
  * [Desktop connection lifecycle](onboarding/connection-lifecycle.md)
  * [Notification lifecycle](onboarding/notification-lifecycle.md)
  * [Bigtable data model](onboarding/bigtable.md)
  * [Observing and debugging](onboarding/observing-and-debugging.md)
  * [Developer workflow](onboarding/developer-workflow.md)
  * [Verification notes](onboarding/verification-notes.md)
* [General Architecture](architecture.md)
  * [Push Reliability Tracking](reliability.md)
  * [Legacy table rotation](table_rotation.md)
* [Install](install.md)
  * [Apple Push Notification (APNs) guide](apns.md)
  * [Google Firebase Cloud Messaging (FCM) guide](fcm.md)
* [Running](running.md)

## Developing
* [Configuring a Stand Alone Development Environment](development.md)
*    [Data Storage Systems](datastore.md)
* [Style](style.md)
* [Testing](testing.md)
* [Release Process](releasing.md)

## Reference

* [HTTP Endpoints for Notifications](http.md)
* [Error codes](errors.md)
* [Glossary](glossary.md)
* [Why rust?](rust.md)
