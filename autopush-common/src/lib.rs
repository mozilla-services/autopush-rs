#![recursion_limit = "1024"]

#[macro_use]
extern crate slog;
#[macro_use]
extern crate slog_scope;

#[macro_use]
pub mod db;
pub mod endpoint;
pub mod errors;
pub mod logging;
pub mod message_id;
pub mod metrics;
pub mod middleware;
pub mod notification;
pub mod sentry;
pub mod tags;
pub mod test_support;

#[macro_use]
pub mod util;

/// Define some global TTLs.
///
/// [RFC8030 notes](https://datatracker.ietf.org/doc/html/rfc8030#section-5.2) that
/// technically these are u32 values, but we should be kind and use u64. The RFC also
/// does not define a maximum TTL duration. Traditionally, Autopush has capped this
/// to 60 days, partly because we used to require monthly message table rotation.
/// (The TTL was given an extra 30 days grace in order to handle dates near the
/// turn of the month, when we might need to look in two tables for the data.)
/// Since we now have automatically applied garbage collection, we are at a bit of
/// liberty about how long these should be.
///
/// That gets back to the concept that Push messages are supposed to be "timely".
/// A user may not appreciate that they have an undelivered calendar reminder from
/// 58 days ago, nor should they be interested in a meeting alert that happened last
/// month. When a User Agent (UA) connects, it receives all pending messages. If
/// a user has not used the User Agent in more than
/// [60 days](https://searchfox.org/mozilla-central/search?q=OFFER_PROFILE_RESET_INTERVAL_MS),
/// the User Agent suggest "refreshing Firefox", which essentially throws away one's
/// current profile. This would include all subscriptions a user may have had.
///
/// To that end, messages left unread for more than 30 days should be considered
/// "abandoned" and any router info assigned to a User Agent that has not contacted
/// Autopush in 60 days can be discarded.
///
const ONE_DAY_IN_SECONDS: u64 = 24 * 60 * 60;
/// The maximum TTL for notifications, 30 days in seconds
pub const MAX_NOTIFICATION_TTL: u64 = 30 * ONE_DAY_IN_SECONDS;
/// FCM has a max TTL of 4 weeks.
pub const MAX_FCM_NOTIFICATION_TTL: u64 = 4 * 7 * ONE_DAY_IN_SECONDS;
/// The maximum TTL for router records, 60 days in seconds
pub const MAX_ROUTER_TTL: u64 = 2 * MAX_NOTIFICATION_TTL;
