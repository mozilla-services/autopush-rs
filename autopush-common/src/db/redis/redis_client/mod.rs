use std::collections::HashSet;
use std::fmt;
use std::fmt::Display;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use cadence::{CountedExt, StatsdClient};
use redis::aio::MultiplexedConnection;
use redis::{AsyncCommands, SetExpiry, SetOptions};
use uuid::Uuid;

use crate::db::{
    client::{DbClient, FetchMessageResponse},
    error::{DbError, DbResult},
    DbSettings, Notification, User, MAX_ROUTER_TTL,
};
use crate::util::ms_since_epoch;

mod error;

use super::RedisDbSettings;

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

struct Chanid<'a>(&'a Uuid);

impl<'a> Display for Chanid<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0.as_hyphenated())
    }
}

impl<'a> From<Chanid<'a>> for String {
    fn from(uaid: Chanid) -> String {
        uaid.0.as_hyphenated().to_string()
    }
}

#[derive(Clone)]
/// Wrapper for the Redis connection
pub struct RedisClientImpl {
    /// Database connector string
    pub client: redis::Client,
    pub conn: Arc<Mutex<Option<MultiplexedConnection>>>,
    pub(crate) settings: RedisDbSettings,
    /// Metrics client
    metrics: Arc<StatsdClient>,
    redis_opts: SetOptions,
}

impl RedisClientImpl {
    pub fn new(metrics: Arc<StatsdClient>, settings: &DbSettings) -> DbResult<Self> {
        debug!("🐰 New redis client");
        let dsn = settings
            .dsn
            .clone()
            .ok_or(DbError::General("Could not find DSN".to_owned()))?;
        let client = redis::Client::open(dsn)?;
        let db_settings = RedisDbSettings::try_from(settings.db_settings.as_ref())?;
        info!("🐰 {:#?}", db_settings);

        Ok(Self {
            client,
            conn: Arc::new(Mutex::new(None)),
            settings: db_settings,
            metrics,
            redis_opts: SetOptions::default().with_expiration(SetExpiry::EX(MAX_ROUTER_TTL)),
        })
    }

    /// Return a [ConnectionLike], which implement redis [Commands] and can be
    /// used in pipes.
    ///
    /// Pools also return a ConnectionLike, so we can add support for pools later.
    async fn connection(&self) -> DbResult<redis::aio::MultiplexedConnection> {
        {
            let conn = self
                .conn
                .lock()
                .map_err(|e| DbError::General(e.to_string()))?
                .clone();

            if let Some(co) = conn {
                return Ok(co);
            }
        }
        let config = if self.settings.timeout.is_some_and(|t| !t.is_zero()) {
            redis::AsyncConnectionConfig::new()
                .set_connection_timeout(self.settings.timeout.unwrap())
        } else {
            redis::AsyncConnectionConfig::new()
        };
        let co = self
            .client
            .get_multiplexed_async_connection_with_config(&config)
            .await
            .map_err(|e| DbError::ConnectionError(format!("Cannot connect to redis: {}", e)))?;
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| DbError::General(e.to_string()))?;
        *conn = Some(co.clone());
        Ok(co)
    }

    fn user_key(&self, uaid: &Uaid) -> String {
        format!("autopush/user/{}", uaid)
    }

    /// This store the last connection record, but doesn't update User
    fn last_co_key(&self, uaid: &Uaid) -> String {
        format!("autopush/co/{}", uaid)
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
}

#[async_trait]
impl DbClient for RedisClientImpl {
    /// add user to the database
    async fn add_user(&self, user: &User) -> DbResult<()> {
        trace!("🐰 Adding user");
        trace!("🐰 Logged at {}", &user.connected_at);
        let mut con = self.connection().await?;
        let uaid = Uaid(&user.uaid);
        let user_key = self.user_key(&uaid);
        let co_key = self.last_co_key(&uaid);
        let _: () = redis::pipe()
            .set_options(co_key, ms_since_epoch(), self.redis_opts)
            .set_options(user_key, serde_json::to_string(user)?, self.redis_opts)
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
            self.add_user(&user).await?;
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
        redis::pipe()
            .del(&user_key)
            .del(&co_key)
            .del(&chan_list_key)
            .del(&msg_list_key)
            .del(&exp_list_key)
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
            .set_options(co_key, ms_since_epoch(), self.redis_opts)
            .exec_async(&mut con)
            .await?;
        Ok(())
    }

    /// Add channels in bulk (used mostly during migration)
    async fn add_channels(&self, uaid: &Uuid, channels: HashSet<Uuid>) -> DbResult<()> {
        let uaid = Uaid(uaid);
        // channel_ids are stored as a set within a single redis key
        let mut con = self.connection().await?;
        let co_key = self.last_co_key(&uaid);
        let chan_list_key = self.channel_list_key(&uaid);
        redis::pipe()
            .set_options(co_key, ms_since_epoch(), self.redis_opts)
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
        let mut con = self.client.get_multiplexed_async_connection().await?;
        //let mut con = self.connection().await?;
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
        let channel_id = Chanid(channel_id);
        let mut con = self.connection().await?;
        let co_key = self.last_co_key(&uaid);
        let chan_list_key = self.channel_list_key(&uaid);
        // Remove {channel_id} from autopush/channel/{auid}
        trace!("🐰 Removing channel {}", channel_id);
        let (status,): (bool,) = redis::pipe()
            .set_options(co_key, ms_since_epoch(), self.redis_opts)
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
        if let Some(mut user) = self.get_user(&uaid).await? {
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
        let msg_key = self.message_key(&uaid, &msg_id);
        // message.ttl is already min(headers.ttl, MAX_NOTIFICATION_TTL)
        // see autoendpoint/src/extractors/notification_headers.rs
        let opts = SetOptions::default().with_expiration(SetExpiry::EX(message.ttl));

        debug!("🐰 Saving message {} :: {:?}", &msg_key, &message);
        trace!(
            "🐰 timestamp: {:?}",
            &message.timestamp.to_be_bytes().to_vec()
        );

        // Remember, `timestamp` is effectively the time to kill the message, not the
        // current time.
        let expiry = (SystemTime::now() + Duration::from_secs(message.ttl))
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        trace!("🐰 Message Expiry {}", expiry);

        let mut pipe = redis::pipe();

        // If this is a topic message:
        // zadd(msg_list_key) and zadd(exp_list_key) will replace their old entry
        // in the hashset if one already exists
        // and set(msg_key, message) will override it too: nothing to do.
        let is_topic = message.topic.is_some();

        // Store notification record in autopush/msg/{aud}/{chidmessageid}
        // And store {chidmessageid} in autopush/msgs/{aud}
        let msg_key = self.message_key(&uaid, &msg_id);
        pipe.set_options(msg_key, serde_json::to_string(&message)?, opts)
            // The function [fecth_timestamp_messages] takes a timestamp in input,
            // here we use the timestamp of the record (in ms)
            .zadd(&exp_list_key, &msg_id, expiry)
            .zadd(&msg_list_key, &msg_id, ms_since_epoch());

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
        // plate simple way of solving this:
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
        let mut con = self.connection().await?;
        let exp_id_list: Vec<String> = con.zrangebyscore(&exp_list_key, 0, timestamp).await?;
        if exp_id_list.len() > 0 {
            trace!("🐰🔥 Deleting {} expired msgs", exp_id_list.len());
            redis::pipe()
                .del(&exp_id_list)
                .zrem(&msg_list_key, &exp_id_list)
                .zrem(&exp_list_key, &exp_id_list)
                .exec_async(&mut con)
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
        let msg_key = self.message_key(&uaid, &chidmessageid);
        let msg_list_key = self.message_list_key(&uaid);
        let exp_list_key = self.message_exp_list_key(&uaid);
        debug!("🐰🔥 Deleting message {}", &msg_key);
        let mut con = self.connection().await?;
        // We remove the id from the exp list at the end, to be sure
        // it can't be removed from the list before the message is removed
        redis::pipe()
            .del(&msg_key)
            .zrem(&msg_list_key, &chidmessageid)
            .zrem(&exp_list_key, &chidmessageid)
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
    /// after [`timestamp`] (millisecs).
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
        trace!("🐰 Fecthing {} messages since {:?}", limit, timestamp);
        let mut con = self.connection().await?;
        let msg_list_key = self.message_list_key(&uaid);
        // ZRANGE Key (x +inf LIMIT 0 limit
        let (messages_id, mut scores): (Vec<String>, Vec<u64>) = con
            .zrangebyscore_limit_withscores::<&str, &str, &str, Vec<(String, u64)>>(
                &msg_list_key,
                &format!("({}", timestamp.unwrap_or(0)),
                "+inf",
                0,
                limit as isize,
            )
            .await?
            .into_iter()
            .map(|(id, s): (String, u64)| (self.message_key(&uaid, &id), s))
            .unzip();
        if messages_id.len() == 0 {
            trace!("🐰 No message found");
            return Ok(FetchMessageResponse {
                messages: vec![],
                timestamp: None,
            });
        }
        let messages: Vec<Notification> = if messages_id.len() == 0 {
            vec![]
        } else {
            con.mget::<&Vec<String>, Vec<Option<String>>>(&messages_id)
                .await?
                .into_iter()
                .filter_map(|opt: Option<String>| {
                    if opt.is_none() {
                        // We return dummy expired event if we can't fetch the said event,
                        // it means the event has expired
                        Some(Notification {
                            timestamp: 1,
                            ..Default::default()
                        })
                    } else {
                        opt.and_then(|m| serde_json::from_str(&m).ok())
                    }
                })
                .collect()
        };
        let timestamp = scores.pop();
        trace!("🐰 Found {} messages until {:?}", messages.len(), timestamp);
        Ok(FetchMessageResponse {
            messages,
            timestamp,
        })
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

    use super::*;
    const TEST_USER: &str = "DEADBEEF-0000-0000-0000-0123456789AB";
    const TEST_CHID: &str = "DECAFBAD-0000-0000-0000-0123456789AB";
    const TOPIC_CHID: &str = "DECAFBAD-1111-0000-0000-0123456789AB";

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn new_client() -> DbResult<RedisClientImpl> {
        let env_dsn = "redis://localhost".into(); // We force localhost to force test environment
        let settings = DbSettings {
            dsn: Some(env_dsn),
            db_settings: "".into(),
        };
        let metrics = Arc::new(StatsdClient::builder("", cadence::NopMetricSink).build());
        RedisClientImpl::new(metrics, &settings)
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

        let uaid = Uuid::parse_str(TEST_USER).unwrap();
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
        let timestamp = now();
        let fetch_timestamp = ms_since_epoch();
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
        client
            .increment_storage(&uaid, fetch_timestamp + 10000)
            .await?;
        let msgs = client.fetch_timestamp_messages(&uaid, None, 999).await?;
        assert_eq!(msgs.messages.len(), 0);
        Ok(())
    }

    /// run a gauntlet of testing. These are a bit linear because they need
    /// to run in sequence.
    #[actix_rt::test]
    async fn run_gauntlet() -> DbResult<()> {
        init_test_logging();
        let client = new_client()?;

        let connected_at = ms_since_epoch();

        let uaid = Uuid::parse_str(TEST_USER).unwrap();
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
        let timestamp = now();
        let sort_key = now();
        // Unlike Bigtable, [fetch_timestamp_messages] uses and return a
        // timestamp in milliseconds
        let fetch_timestamp = ms_since_epoch();
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

        // Grab all 1 of the messages that were submmited within the past 10 seconds.
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
        let timestamp = now();
        let sort_key = now();

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
            timestamp: now(),
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

        // Grab the message that was submmited.
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
}
