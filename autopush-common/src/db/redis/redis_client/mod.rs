use std::collections::HashSet;
use std::fmt;
use std::fmt::Display;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use cadence::{CountedExt, StatsdClient};
use redis::aio::MultiplexedConnection;
use redis::{AsyncCommands, SetExpiry, SetOptions};
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::db::{
    client::{DbClient, FetchMessageResponse},
    error::{DbError, DbResult},
    DbSettings, Notification, User,
};
use crate::util::{ms_since_epoch, sec_since_epoch};

use super::RedisDbSettings;

fn now_secs() -> u64 {
    // Return the current time in seconds since EPOCH
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}


/// Semi convenience wrapper to ensure that the UAID is formatted and displayed consistently.
struct Uaid<'a>(&'a Uuid);

impl<'a> Display for Uaid<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0.as_hyphenated())
    }
}

impl<'a> From<Uaid<'a>> for String {
    fn from(uaid: Uaid) -> String {
        uaid.0.as_hyphenated().to_string()
    }
}

struct ChannelId<'a>(&'a Uuid);

impl<'a> Display for ChannelId<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0.as_hyphenated())
    }
}

impl<'a> From<ChannelId<'a>> for String {
    fn from(chid: ChannelId) -> String {
        chid.0.as_hyphenated().to_string()
    }
}

#[derive(Clone)]
/// Wrapper for the Redis connection
pub struct RedisClientImpl {
    /// Database connector string
    pub client: redis::Client,
    pub conn: OnceCell<MultiplexedConnection>,
    pub(crate) timeout: Option<Duration>,
    /// Metrics client
    metrics: Arc<StatsdClient>,
    router_opts: SetOptions,
    // Default notification options (mostly TTL)
    notification_opts: SetOptions,
}

impl RedisClientImpl {
    pub fn new(metrics: Arc<StatsdClient>, settings: &DbSettings) -> DbResult<Self> {
        debug!("🐰 New redis client");
        let dsn = settings.dsn.clone().ok_or(DbError::General(
            "Redis DSN not configured. Set `db_dsn` to `redis://HOST:PORT` in settings.".to_owned(),
        ))?;
        let client = redis::Client::open(dsn)?;
        let db_settings = RedisDbSettings::try_from(settings.db_settings.as_ref())?;
        info!("🐰 {:#?}", db_settings);
        let router_ttl_secs = db_settings.router_ttl.unwrap_or_default().as_secs();
        let notification_ttl_secs = db_settings.notification_ttl.unwrap_or_default().as_secs();
        // We specify different TTLs for router vs message.
        Ok(Self {
            client,
            conn: OnceCell::new(),
            timeout: db_settings.timeout,
            metrics,
            router_opts: SetOptions::default().with_expiration(SetExpiry::EX(router_ttl_secs)),
            notification_opts: SetOptions::default()
                .with_expiration(SetExpiry::EX(notification_ttl_secs)),
        })
    }

    /// Return a [ConnectionLike], which implement redis [Commands] and can be
    /// used in pipes.
    ///
    /// Pools also return a ConnectionLike, so we can add support for pools later.
    async fn connection(&self) -> DbResult<redis::aio::MultiplexedConnection> {
        let config = if self.timeout.is_some_and(|t| !t.is_zero()) {
            redis::AsyncConnectionConfig::new().set_connection_timeout(self.timeout.unwrap())
        } else {
            redis::AsyncConnectionConfig::new()
        };
        Ok(self
            .conn
            .get_or_try_init(|| async {
                self.client
                    .get_multiplexed_async_connection_with_config(&config.clone())
                    .await
            })
            .await
            .inspect_err(|e| error!("No Redis Connection available: {}", e.to_string()))?
            .clone())
    }

    fn user_key(&self, uaid: &Uaid) -> String {
        format!("autopush/user/{}", uaid)
    }

    /// This store the last connection record, but doesn't update User
    fn last_co_key(&self, uaid: &Uaid) -> String {
        format!("autopush/co/{}", uaid)
    }

    /// This store the last timestamp incremented by the server once messages are ACK'ed
    fn storage_timestamp_key(&self, uaid: &Uaid) -> String {
        format!("autopush/timestamp/{}", uaid)
    }

    fn channel_list_key(&self, uaid: &Uaid) -> String {
        format!("autopush/channels/{}", uaid)
    }

    fn message_list_key(&self, uaid: &Uaid) -> String {
        format!("autopush/msgs/{}", uaid)
    }

    fn message_exp_list_key(&self, uaid: &Uaid) -> String {
        format!("autopush/msgs_exp/{}", uaid)
    }

    fn message_key(&self, uaid: &Uaid, chidmessageid: &str) -> String {
        format!("autopush/msg/{}/{}", uaid, chidmessageid)
    }

    #[cfg(feature = "reliable_report")]
    fn reliability_key(
        &self,
        reliability_id: &str,
        state: &crate::reliability::ReliabilityState,
    ) -> String {
        format!("autopush/reliability/{}/{}", reliability_id, state)
    }

    #[cfg(test)]
    /// Return a single "raw" message (used by testing and validation)
    async fn fetch_message(&self, uaid: &Uuid, chidmessageid: &str) -> DbResult<Option<String>> {
        let message_key = self.message_key(&Uaid(uaid), chidmessageid);
        let mut con = self.connection().await?;
        debug!("🐰 Fetching message from {}", &message_key);
        let message = con.get::<String, Option<String>>(message_key).await?;
        Ok(message)

    }
}

#[async_trait]
impl DbClient for RedisClientImpl {
    /// add user to the database
    async fn add_user(&self, user: &User) -> DbResult<()> {
        let uaid = Uaid(&user.uaid);
        let user_key = self.user_key(&uaid);
        let mut con = self.connection().await?;
        let co_key = self.last_co_key(&uaid);
        trace!("🐰 Adding user {} at {}:{}", &user.uaid, &user_key, &co_key);
        trace!("🐰 Logged at {}", &user.connected_at);
        redis::pipe()
            .set_options(co_key, ms_since_epoch(), self.router_opts)
            .set_options(user_key, serde_json::to_string(user)?, self.router_opts)
            .exec_async(&mut con)
            .await?;
        Ok(())
    }

    /// To update the TTL of the Redis entry we just have to SET again, with the new expiry
    ///
    /// NOTE: This function is called by mobile during the daily
    /// [autoendpoint::routes::update_token_route] handling, and by desktop
    /// [autoconnect-ws-sm::get_or_create_user]` which is called
    /// during the `HELLO` handler. This should be enough to ensure that the ROUTER records
    /// are properly refreshed for "lively" clients.
    ///
    /// NOTE: There is some, very small, potential risk that a desktop client that can
    /// somehow remain connected the duration of MAX_ROUTER_TTL, may be dropped as not being
    /// "lively".
    async fn update_user(&self, user: &mut User) -> DbResult<bool> {
        trace!("🐰 Updating user");
        let mut con = self.connection().await?;
        let co_key = self.last_co_key(&Uaid(&user.uaid));
        let last_co: Option<u64> = con.get(&co_key).await?;
        if last_co.is_some_and(|c| c < user.connected_at) {
            trace!(
                "🐰 Was connected at {}, now at {}",
                last_co.unwrap(),
                &user.connected_at
            );
            self.add_user(user).await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn get_user(&self, uaid: &Uuid) -> DbResult<Option<User>> {
        let mut con = self.connection().await?;
        let user_key = self.user_key(&Uaid(uaid));
        let user: Option<User> = con
            .get::<&str, Option<String>>(&user_key)
            .await?
            .and_then(|s| serde_json::from_str(s.as_ref()).ok());
        if user.is_some() {
            trace!("🐰 Found a record for {}", &uaid);
        }
        Ok(user)
    }

    async fn remove_user(&self, uaid: &Uuid) -> DbResult<()> {
        let uaid = Uaid(uaid);
        let mut con = self.connection().await?;
        let user_key = self.user_key(&uaid);
        let co_key = self.last_co_key(&uaid);
        let chan_list_key = self.channel_list_key(&uaid);
        let msg_list_key = self.message_list_key(&uaid);
        let exp_list_key = self.message_exp_list_key(&uaid);
        let timestamp_key = self.storage_timestamp_key(&uaid);
        redis::pipe()
            .del(&user_key)
            .del(&co_key)
            .del(&chan_list_key)
            .del(&msg_list_key)
            .del(&exp_list_key)
            .del(&timestamp_key)
            .exec_async(&mut con)
            .await?;
        Ok(())
    }

    async fn add_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<()> {
        let uaid = Uaid(uaid);
        let mut con = self.connection().await?;
        let co_key = self.last_co_key(&uaid);
        let chan_list_key = self.channel_list_key(&uaid);

        let _: () = redis::pipe()
            .rpush(chan_list_key, channel_id.as_hyphenated().to_string())
            .set_options(co_key, ms_since_epoch(), self.router_opts)
            .exec_async(&mut con)
            .await?;
        Ok(())
    }

    /// Add channels in bulk (used mostly during migration)
    async fn add_channels(&self, uaid: &Uuid, channels: HashSet<Uuid>) -> DbResult<()> {
        let uaid = Uaid(uaid);
        // channel_ids are stored a list within a single redis key
        let mut con = self.connection().await?;
        let co_key = self.last_co_key(&uaid);
        let chan_list_key = self.channel_list_key(&uaid);
        redis::pipe()
            .set_options(co_key, ms_since_epoch(), self.router_opts)
            .rpush(
                chan_list_key,
                channels
                    .into_iter()
                    .map(|c| c.as_hyphenated().to_string())
                    .collect::<Vec<String>>(),
            )
            .exec_async(&mut con)
            .await?;
        Ok(())
    }

    async fn get_channels(&self, uaid: &Uuid) -> DbResult<HashSet<Uuid>> {
        let uaid = Uaid(uaid);
        let mut con = self.connection().await?;
        let chan_list_key = self.channel_list_key(&uaid);
        let channels: HashSet<Uuid> = con
            .lrange::<&str, HashSet<String>>(&chan_list_key, 0, -1)
            .await?
            .into_iter()
            .filter_map(|s| Uuid::from_str(&s).ok())
            .collect();
        trace!("🐰 Found {} channels for {}", channels.len(), &uaid);
        Ok(channels)
    }

    /// Delete the channel. Does not delete its associated pending messages.
    async fn remove_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<bool> {
        let uaid = Uaid(uaid);
        let channel_id = ChannelId(channel_id);
        let mut con = self.connection().await?;
        let co_key = self.last_co_key(&uaid);
        let chan_list_key = self.channel_list_key(&uaid);
        // Remove {channel_id} from autopush/channel/{auid}
        trace!("🐰 Removing channel {}", channel_id);
        let (status,): (bool,) = redis::pipe()
            .set_options(co_key, ms_since_epoch(), self.router_opts)
            .ignore()
            .lrem(&chan_list_key, 1, channel_id.to_string())
            .query_async(&mut con)
            .await?;
        Ok(status)
    }

    /// Remove the node_id
    async fn remove_node_id(
        &self,
        uaid: &Uuid,
        _node_id: &str,
        _connected_at: u64,
        _version: &Option<Uuid>,
    ) -> DbResult<bool> {
        if let Some(mut user) = self.get_user(uaid).await? {
            user.node_id = None;
            self.update_user(&mut user).await?;
        }
        Ok(true)
    }

    /// Write the notification to storage.
    ///
    /// If the message contains a topic, we remove the old message
    async fn save_message(&self, uaid: &Uuid, message: Notification) -> DbResult<()> {
        let uaid = Uaid(uaid);
        let mut con = self.connection().await?;
        let msg_list_key = self.message_list_key(&uaid);
        let exp_list_key = self.message_exp_list_key(&uaid);
        let msg_id = &message.chidmessageid();
        let msg_key = self.message_key(&uaid, msg_id);

        debug!("🐰 Saving message {} :: {:?}", &msg_key, &message);
        trace!(
            "🐰 timestamp: {:?}",
            &message.timestamp.to_be_bytes().to_vec()
        );

        // Remember, `timestamp` is effectively the time to kill the message, not the
        // current time.
        let expiry = now_secs() + message.ttl;
        trace!("🐰 Message Expiry {}, currently:{} ", expiry, now_secs());

        let mut pipe = redis::pipe();

        // If this is a topic message:
        // zadd(msg_list_key) and zadd(exp_list_key) will replace their old entry
        // in the sorted set if one already exists
        // and set(msg_key, message) will override it too: nothing to do.
        let is_topic = message.topic.is_some();

        // notification_ttl is already min(headers.ttl, MAX_NOTIFICATION_TTL)
        // see autoendpoint/src/extractors/notification_headers.rs
        let notif_opts = self
            .notification_opts
            .with_expiration(SetExpiry::EXAT(expiry));

        // Store notification record in autopush/msg/{uaid}/{chidmessageid}
        // And store {chidmessageid} in autopush/msgs/{uaid}
        debug!("🐰 Saving to {}", &msg_key);
        pipe.set_options(msg_key, serde_json::to_string(&message)?, notif_opts)
            // The function [fetch_timestamp_messages] takes a timestamp in input,
            // here we use the timestamp of the record
            .zadd(&exp_list_key, msg_id, expiry)
            .zadd(&msg_list_key, msg_id, sec_since_epoch());

        let _: () = pipe.exec_async(&mut con).await?;
        self.metrics
            .incr_with_tags("notification.message.stored")
            .with_tag("topic", &is_topic.to_string())
            .with_tag("database", &self.name())
            .send();
        Ok(())
    }

    /// Save a batch of messages to the database.
    ///
    /// Currently just iterating through the list and saving one at a time. There's a bulk way
    /// to save messages, but there are other considerations (e.g. mutation limits)
    async fn save_messages(&self, uaid: &Uuid, messages: Vec<Notification>) -> DbResult<()> {
        // A plate simple way of solving this:
        for message in messages {
            self.save_message(uaid, message).await?;
        }
        Ok(())
    }

    /// Delete expired messages
    async fn increment_storage(&self, uaid: &Uuid, timestamp: u64) -> DbResult<()> {
        let uaid = Uaid(uaid);
        debug!("🐰🔥 Incrementing storage to {}", timestamp);
        let msg_list_key = self.message_list_key(&uaid);
        let exp_list_key = self.message_exp_list_key(&uaid);
        let storage_timestamp_key = self.storage_timestamp_key(&uaid);
        let mut con = self.connection().await?;
        trace!("🐇 SEARCH: increment: {:?} - {}", &exp_list_key, timestamp);
        let exp_id_list: Vec<String> = con.zrangebyscore(&exp_list_key, 0, timestamp).await?;
        if !exp_id_list.is_empty() {
            // Remember, we store just the message_ids in the exp and msg lists, but need to convert back to 
            // the full message keys for deletion.
            let delete_msg_keys:Vec<String> = exp_id_list.clone().into_iter().map(|msg_id| self.message_key(&uaid, &msg_id)).collect();

            trace!("🐰🔥:rem: Deleting {} : [{:?}]", msg_list_key, &delete_msg_keys);
            trace!("🐰🔥:rem: Deleting {} : [{:?}]", exp_list_key, &exp_id_list);
            redis::pipe()
                .set_options::<_, _>(&storage_timestamp_key, timestamp, self.router_opts)
                .del(&delete_msg_keys)
                .zrem(&msg_list_key, &exp_id_list)
                .zrem(&exp_list_key, &exp_id_list)
                .exec_async(&mut con)
                .await?;
        } else {
            con.set_options::<_, _, ()>(&storage_timestamp_key, timestamp, self.router_opts)
                .await?;
        }
        Ok(())
    }

    /// Delete the notification from storage.
    async fn remove_message(&self, uaid: &Uuid, chidmessageid: &str) -> DbResult<()> {
        let uaid = Uaid(uaid);
        trace!(
            "🐰 attemping to delete {:?} :: {:?}",
            uaid.to_string(),
            chidmessageid
        );
        let msg_key = self.message_key(&uaid, chidmessageid);
        let msg_list_key = self.message_list_key(&uaid);
        let exp_list_key = self.message_exp_list_key(&uaid);
        debug!("🐰🔥 Deleting message {}", &msg_key);
        let mut con = self.connection().await?;
        // We remove the id from the exp list at the end, to be sure
        // it can't be removed from the list before the message is removed
        trace!(
            "🐰🔥:remsg: Deleting {} : {:?}",
            msg_list_key,
            &chidmessageid
        );
        trace!(
            "🐰🔥:remsg: Deleting {} : {:?}",
            exp_list_key,
            &chidmessageid
        );
        redis::pipe()
            .del(&msg_key)
            .zrem(&msg_list_key, chidmessageid)
            .zrem(&exp_list_key, chidmessageid)
            .exec_async(&mut con)
            .await?;
        self.metrics
            .incr_with_tags("notification.message.deleted")
            .with_tag("database", &self.name())
            .send();
        Ok(())
    }

    /// Topic messages are handled as other messages with redis, we return nothing.
    async fn fetch_topic_messages(
        &self,
        _uaid: &Uuid,
        _limit: usize,
    ) -> DbResult<FetchMessageResponse> {
        Ok(FetchMessageResponse {
            messages: vec![],
            timestamp: None,
        })
    }

    /// Return [`limit`] messages pending for a [`uaid`] that have a record timestamp
    /// after [`timestamp`] (secs).
    ///
    /// If [`limit`] = 0, we fetch all messages after [`timestamp`].
    ///
    /// This can return expired messages, following bigtables behavior
    async fn fetch_timestamp_messages(
        &self,
        uaid: &Uuid,
        timestamp: Option<u64>,
        limit: usize,
    ) -> DbResult<FetchMessageResponse> {
        let uaid = Uaid(uaid);
        trace!("🐰 Fetching {} messages since {:?}", limit, timestamp);
        let mut con = self.connection().await?;
        let msg_list_key = self.message_list_key(&uaid);
        let timestamp = if let Some(timestamp) = timestamp {
            timestamp
        } else {
            let storage_timestamp_key = self.storage_timestamp_key(&uaid);
            con.get(&storage_timestamp_key).await.unwrap_or(0)
        };
        // ZRANGE Key (x) +inf LIMIT 0 limit
        trace!(
            "🐇 SEARCH: zrangebyscore {:?} {} +inf withscores limit 0 {:?}",
            &msg_list_key,
            timestamp,
            limit,
        );
        let results = con
            .zrangebyscore_limit_withscores::<&str, &str, &str, Vec<(String, u64)>>(
                &msg_list_key,
                &timestamp.to_string(),
                "+inf",
                0,
                limit as isize,
            )
            .await?;
        let (messages_id, mut scores): (Vec<String>, Vec<u64>) = results
            .into_iter()
            .map(|(id, s): (String, u64)| (self.message_key(&uaid, &id), s))
            .unzip();
        if messages_id.is_empty() {
            trace!("🐰 No message found");
            return Ok(FetchMessageResponse {
                messages: vec![],
                timestamp: None,
            });
        }
        let messages: Vec<Notification> = con
            .mget::<&Vec<String>, Vec<Option<String>>>(&messages_id)
            .await?
            .into_iter()
            .filter_map(|opt: Option<String>| {
                if let Some(m) = opt {
                    serde_json::from_str(&m)
                        .inspect_err(|e| {
                            // Since we can't raise the error here, at least record it
                            // so that it's not lost.
                            // Mind you, if there is an error here, it's probably due to
                            // some developmental issue since the unit and integration tests
                            // should fail.
                            error!("🐰 ERROR parsing entry: {:?}", e);
                        })
                        .ok()
                } else {
                    None
                }
            })
            .collect();
        if messages.is_empty() {
            trace!("🐰 No Valid messages found");
            return Ok(FetchMessageResponse {
                timestamp: None,
                messages: vec![],
            });
        }
        let timestamp = scores.pop();
        trace!("🐰 Found {} messages until {:?}", messages.len(), timestamp);
        Ok(FetchMessageResponse {
            messages,
            timestamp,
        })
    }

    #[cfg(feature = "reliable_report")]
    async fn log_report(
        &self,
        reliability_id: &str,
        state: crate::reliability::ReliabilityState,
    ) -> DbResult<()> {
        use crate::MAX_NOTIFICATION_TTL_SECS;

        trace!("🐰 Logging reliability report");
        let mut con = self.connection().await?;
        // TODO: Should this be a hash key per reliability_id?
        let reliability_key = self.reliability_key(reliability_id, &state);
        // Reports should last about as long as the notifications they're tied to. 
        let expiry = MAX_NOTIFICATION_TTL_SECS;
        let opts = SetOptions::default().with_expiration(SetExpiry::EX(expiry));
        let mut pipe = redis::pipe();
        pipe.set_options(reliability_key, sec_since_epoch(), opts)
            .exec_async(&mut con)
            .await?;
        Ok(())
    }

    async fn health_check(&self) -> DbResult<bool> {
        let mut con = self.connection().await?;
        let _: () = con.ping().await?;
        Ok(true)
    }

    /// Returns true, because there's no table in Redis
    async fn router_table_exists(&self) -> DbResult<bool> {
        Ok(true)
    }

    /// Returns true, because there's no table in Redis
    async fn message_table_exists(&self) -> DbResult<bool> {
        Ok(true)
    }

    fn box_clone(&self) -> Box<dyn DbClient> {
        Box::new(self.clone())
    }

    fn name(&self) -> String {
        "Redis".to_owned()
    }

    fn pool_status(&self) -> Option<deadpool::Status> {
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::{logging::init_test_logging, util::ms_since_epoch};
    use std::env;
    use rand::prelude::*;

    use super::*;
    const TEST_CHID: &str = "DECAFBAD-0000-0000-0000-0123456789AB";
    const TOPIC_CHID: &str = "DECAFBAD-1111-0000-0000-0123456789AB";

    fn new_client() -> DbResult<RedisClientImpl> {
        // Use an environment variable to potentially override the default redis test host.
        let host = env::var("REDIS_HOST").unwrap_or("localhost".into());
        let env_dsn = format!("redis://{host}");
        debug!("🐰 Connecting to {env_dsn}");
        let settings = DbSettings {
            dsn: Some(env_dsn),
            db_settings: "".into(),
        };
        let metrics = Arc::new(StatsdClient::builder("", cadence::NopMetricSink).build());
        RedisClientImpl::new(metrics, &settings)
    }

    fn gen_test_user() -> String {
        // Create a semi-unique test user to avoid conflicting test values.
        let mut rng = rand::rng();
        let test_num = rng.random::<u8>();
        format!("DEADBEEF-0000-0000-{:04}-{:012}",test_num, now_secs())
    }

    #[actix_rt::test]
    async fn health_check() {
        let client = new_client().unwrap();

        let result = client.health_check().await;
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    /// Test if [increment_storage] correctly wipe expired messages
    #[actix_rt::test]
    async fn wipe_expired() -> DbResult<()> {
        init_test_logging();
        let client = new_client()?;

        let connected_at = ms_since_epoch();

        let uaid = Uuid::parse_str(&gen_test_user()).unwrap();
        let chid = Uuid::parse_str(TEST_CHID).unwrap();

        let node_id = "test_node".to_owned();

        // purge the user record if it exists.
        let _ = client.remove_user(&uaid).await;

        let test_user = User {
            uaid,
            router_type: "webpush".to_owned(),
            connected_at,
            router_data: None,
            node_id: Some(node_id.clone()),
            ..Default::default()
        };

        // purge the old user (if present)
        // in case a prior test failed for whatever reason.
        let _ = client.remove_user(&uaid).await;

        // can we add the user?
        let timestamp = now_secs();
        client.add_user(&test_user).await?;
        let test_notification = crate::db::Notification {
            channel_id: chid,
            version: "test".to_owned(),
            ttl: 1,
            timestamp,
            data: Some("Encrypted".into()),
            sortkey_timestamp: Some(timestamp),
            ..Default::default()
        };
        client.save_message(&uaid, test_notification).await?;
        client.increment_storage(&uaid, timestamp + 1).await?;
        let msgs = client.fetch_timestamp_messages(&uaid, None, 999).await?;
        assert_eq!(msgs.messages.len(), 0);
        assert!(client.remove_user(&uaid).await.is_ok());
        Ok(())
    }

    /// run a gauntlet of testing. These are a bit linear because they need
    /// to run in sequence.
    #[actix_rt::test]
    async fn run_gauntlet() -> DbResult<()> {
        init_test_logging();
        let client = new_client()?;

        let connected_at = ms_since_epoch();

        let user_id = &gen_test_user();
        let uaid = Uuid::parse_str(&user_id).unwrap();
        let chid = Uuid::parse_str(TEST_CHID).unwrap();
        let topic_chid = Uuid::parse_str(TOPIC_CHID).unwrap();

        let node_id = "test_node".to_owned();

        // purge the user record if it exists.
        let _ = client.remove_user(&uaid).await;

        let test_user = User {
            uaid,
            router_type: "webpush".to_owned(),
            connected_at,
            router_data: None,
            node_id: Some(node_id.clone()),
            ..Default::default()
        };

        // purge the old user (if present)
        // in case a prior test failed for whatever reason.
        let _ = client.remove_user(&uaid).await;

        // can we add the user?
        client.add_user(&test_user).await?;
        let fetched = client.get_user(&uaid).await?;
        assert!(fetched.is_some());
        let fetched = fetched.unwrap();
        assert_eq!(fetched.router_type, "webpush".to_owned());

        // Simulate a connected_at occuring before the following writes
        let connected_at = ms_since_epoch();

        // can we add channels?
        client.add_channel(&uaid, &chid).await?;
        let channels = client.get_channels(&uaid).await?;
        assert!(channels.contains(&chid));

        // can we add lots of channels?
        let mut new_channels: HashSet<Uuid> = HashSet::new();
        new_channels.insert(chid);
        for _ in 1..10 {
            new_channels.insert(uuid::Uuid::new_v4());
        }
        let chid_to_remove = uuid::Uuid::new_v4();
        new_channels.insert(chid_to_remove);
        client.add_channels(&uaid, new_channels.clone()).await?;
        let channels = client.get_channels(&uaid).await?;
        assert_eq!(channels, new_channels);

        // can we remove a channel?
        assert!(client.remove_channel(&uaid, &chid_to_remove).await?);
        assert!(!client.remove_channel(&uaid, &chid_to_remove).await?);
        new_channels.remove(&chid_to_remove);
        let channels = client.get_channels(&uaid).await?;
        assert_eq!(channels, new_channels);

        // now ensure that we can update a user that's after the time we set
        // prior. first ensure that we can't update a user that's before the
        // time we set prior to the last write
        let mut updated = User {
            connected_at,
            ..test_user.clone()
        };
        let result = client.update_user(&mut updated).await;
        assert!(result.is_ok());
        assert!(!result.unwrap());

        // Make sure that the `connected_at` wasn't modified
        let fetched2 = client.get_user(&fetched.uaid).await?.unwrap();
        assert_eq!(fetched.connected_at, fetched2.connected_at);

        // and make sure we can update a record with a later connected_at time.
        let mut updated = User {
            connected_at: fetched.connected_at + 300,
            ..fetched2
        };
        let result = client.update_user(&mut updated).await;
        assert!(result.is_ok());
        assert!(result.unwrap());
        assert_ne!(
            fetched2.connected_at,
            client.get_user(&uaid).await?.unwrap().connected_at
        );

        // can we increment the storage for the user?
        client
            .increment_storage(
                &fetched.uaid,
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            )
            .await?;

        let test_data = "An_encrypted_pile_of_crap".to_owned();
        let timestamp = now_secs();
        let sort_key = now_secs();
        let fetch_timestamp = timestamp;
        // Can we store a message?
        let test_notification = crate::db::Notification {
            channel_id: chid,
            version: "test".to_owned(),
            ttl: 300,
            timestamp,
            data: Some(test_data.clone()),
            sortkey_timestamp: Some(sort_key),
            ..Default::default()
        };
        let res = client.save_message(&uaid, test_notification.clone()).await;
        assert!(res.is_ok());

        let mut fetched = client.fetch_timestamp_messages(&uaid, None, 999).await?;
        assert_ne!(fetched.messages.len(), 0);
        let fm = fetched.messages.pop().unwrap();
        assert_eq!(fm.channel_id, test_notification.channel_id);
        assert_eq!(fm.data, Some(test_data));

        // Grab all 1 of the messages that were submitted within the past 10 seconds.
        let fetched = client
            .fetch_timestamp_messages(&uaid, Some(fetch_timestamp - 10), 999)
            .await?;
        assert_ne!(fetched.messages.len(), 0);

        // Try grabbing a message for 10 seconds from now.
        let fetched = client
            .fetch_timestamp_messages(&uaid, Some(fetch_timestamp + 10), 999)
            .await?;
        assert_eq!(fetched.messages.len(), 0);

        // can we clean up our toys?
        assert!(client
            .remove_message(&uaid, &test_notification.chidmessageid())
            .await
            .is_ok());

        assert!(client.remove_channel(&uaid, &chid).await.is_ok());

        let msgs = client
            .fetch_timestamp_messages(&uaid, None, 999)
            .await?
            .messages;
        assert!(msgs.is_empty());

        // Now, can we do all that with topic messages
        // Unlike bigtable, we don't use [fetch_topic_messages]: it always return None:
        // they are handled as usuals messages.
        client.add_channel(&uaid, &topic_chid).await?;
        let test_data = "An_encrypted_pile_of_crap_with_a_topic".to_owned();
        let timestamp = now_secs();
        let sort_key = now_secs();

        // We store 2 messages, with a single topic
        let test_notification_0 = crate::db::Notification {
            channel_id: topic_chid,
            version: "version0".to_owned(),
            ttl: 300,
            topic: Some("topic".to_owned()),
            timestamp,
            data: Some(test_data.clone()),
            sortkey_timestamp: Some(sort_key),
            ..Default::default()
        };
        assert!(client
            .save_message(&uaid, test_notification_0.clone())
            .await
            .is_ok());

        let test_notification = crate::db::Notification {
            timestamp: now_secs(),
            version: "version1".to_owned(),
            sortkey_timestamp: Some(sort_key + 10),
            ..test_notification_0
        };

        assert!(client
            .save_message(&uaid, test_notification.clone())
            .await
            .is_ok());

        let mut fetched = client.fetch_timestamp_messages(&uaid, None, 999).await?;
        assert_eq!(fetched.messages.len(), 1);
        let fm = fetched.messages.pop().unwrap();
        assert_eq!(fm.channel_id, test_notification.channel_id);
        assert_eq!(fm.data, Some(test_data));

        // Grab the message that was submitted.
        let fetched = client.fetch_timestamp_messages(&uaid, None, 999).await?;
        assert_ne!(fetched.messages.len(), 0);

        // can we clean up our toys?
        assert!(client
            .remove_message(&uaid, &test_notification.chidmessageid())
            .await
            .is_ok());

        assert!(client.remove_channel(&uaid, &topic_chid).await.is_ok());

        let msgs = client
            .fetch_timestamp_messages(&uaid, None, 999)
            .await?
            .messages;
        assert!(msgs.is_empty());

        let fetched = client.get_user(&uaid).await?.unwrap();
        assert!(client
            .remove_node_id(&uaid, &node_id, connected_at, &fetched.version)
            .await
            .is_ok());
        // did we remove it?
        let fetched = client.get_user(&uaid).await?.unwrap();
        assert_eq!(fetched.node_id, None);

        assert!(client.remove_user(&uaid).await.is_ok());

        assert!(client.get_user(&uaid).await?.is_none());
        Ok(())
    }

    #[actix_rt::test]
    async fn test_expiry() -> DbResult<()> {
        // Make sure that we really are purging messages correctly
        init_test_logging();
        let client = new_client()?;

        let uaid = Uuid::parse_str(&gen_test_user()).unwrap();
        let chid = Uuid::parse_str(TEST_CHID).unwrap();
        let now = now_secs();
        
        let test_notification =  crate::db::Notification {
            channel_id: chid,
            version: "test".to_owned(),
            ttl: 2,
            timestamp:now,
            data: Some("SomeData".into()),
            sortkey_timestamp: Some(now),
            ..Default::default()
        };
        debug!("Writing test notif");
        let res = client.save_message(&uaid, test_notification.clone()).await;
        assert!(res.is_ok());
        let key = client.message_key(&Uaid(&uaid), &test_notification.chidmessageid());
        debug!("Checking {}...", &key);
        let msg  = client.fetch_message(&uaid, &test_notification.chidmessageid()).await?;
        assert!(!msg.unwrap().is_empty());
        debug!("Purging...");
        client.increment_storage(&uaid, now+2).await?;
        debug!("Checking {}...", &key);
        let cc = client.fetch_message(&uaid, &test_notification.chidmessageid()).await;
        assert_eq!(cc.unwrap(), None);
        // clean up after the test.
        assert!(client.remove_user(&uaid).await.is_ok());
        Ok(())

    }
}
