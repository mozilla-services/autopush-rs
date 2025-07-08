/// Push Reliability Recorder
///
/// This allows us to track messages from select, known parties (currently, just
/// mozilla generated and consumed) so that we can identify potential trouble spots
/// and where messages expire early. Message expiration can lead to message loss
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::HttpResponse;
use cadence::StatsdClient;
use chrono::TimeDelta;
use deadpool_redis::Config;

use prometheus_client::{
    encoding::text::encode, metrics::family::Family, metrics::gauge::Gauge, registry::Registry,
};
use redis::Pipeline;
use redis::{aio::ConnectionLike, AsyncCommands};

use crate::db::client::DbClient;
use crate::errors::{ApcError, ApcErrorKind, Result};
use crate::metric_name::MetricName;
use crate::metrics::StatsdClientExt;
use crate::util::timing::sec_since_epoch;

// Redis Keys
pub const COUNTS: &str = "state_counts";
pub const EXPIRY: &str = "expiry";

const CONNECTION_EXPIRATION: TimeDelta = TimeDelta::seconds(10);

/// The various states that a message may transit on the way from reception to delivery.
// Note: "Message" in this context refers to the Subscription Update.
// TODO: Differentiate between "transmitted via webpush" and "transmitted via bridge"?
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    PartialOrd,
    Eq,
    serde::Deserialize,
    serde::Serialize,
    strum::Display,
    strum::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ReliabilityState {
    Received = 0,      // Message was received by the Push Server
    Stored,            // Message was stored because it could not be delivered immediately
    Retrieved,         // Message was taken from storage for delivery
    IntTransmitted,    // Message was handed off between autoendpoint and autoconnect
    IntAccepted,       // Message was accepted by autoconnect from autopendpoint
    BridgeTransmitted, // Message was handed off to a mobile bridge for eventual delivery
    Transmitted,       // Message was handed off for delivery to the UA
    Accepted,          // Message was accepted for delivery by the UA
    Delivered,         // Message was provided to the WebApp recipient by the UA
    DecryptionError,   // Message was provided to the UA and it reported a decryption error
    NotDelivered,      // Message was provided to the UA and it reported a not delivered error
    Expired,           // Message expired naturally (e.g. TTL=0)
    Errored,           // Message resulted in an Error state and failed to be delivered.
}

impl ReliabilityState {
    ///  Has the message reached a state where no further transitions should be possible?
    pub fn is_terminal(self) -> bool {
        // NOTE: this list should match the daily snapshot captured by `reliability_cron.py`
        // which will trim these counts after the max message TTL has expired.
        matches!(
            self,
            ReliabilityState::DecryptionError
                | ReliabilityState::BridgeTransmitted
                | ReliabilityState::Delivered
                | ReliabilityState::Errored
                | ReliabilityState::Expired
                | ReliabilityState::NotDelivered
        )
    }
}

#[derive(Clone)]
pub struct PushReliability {
    pool: Option<deadpool_redis::Pool>,
    db: Box<dyn DbClient>,
    metrics: Arc<StatsdClient>,
    retries: usize,
}

// Define a struct to hold the expiry key, since it's easy to flub the order.
pub struct ExpiryKey {
    pub id: String,
    pub state: ReliabilityState,
}

impl std::fmt::Display for ExpiryKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}#{}", self.id, self.state)
    }
}

impl TryFrom<String> for ExpiryKey {
    type Error = ApcError;
    fn try_from(value: String) -> Result<Self> {
        let (id, state) = value.split_once('#').ok_or_else(|| {
            ApcErrorKind::GeneralError("ExpiryKey must be in the format 'id#state'".to_owned())
        })?;
        let state: ReliabilityState = ReliabilityState::from_str(state).map_err(|_| {
            ApcErrorKind::GeneralError(
                "Invalid state in ExpiryKey, must be a valid ReliabilityState".to_owned(),
            )
        })?;
        Ok(Self {
            id: id.to_owned(),
            state,
        })
    }
}

impl PushReliability {
    // Do the magic to make a report instance, whatever that will be.
    pub fn new(
        reliability_dsn: &Option<String>,
        db: Box<dyn DbClient>,
        metrics: &Arc<StatsdClient>,
        retries: usize,
    ) -> Result<Self> {
        let Some(reliability_dsn) = reliability_dsn else {
            debug!("🔍 No reliability DSN declared.");
            return Ok(Self {
                pool: None,
                db: db.clone(),
                metrics: metrics.clone(),
                retries,
            });
        };

        let config: deadpool_redis::Config = Config::from_url(reliability_dsn);
        let pool = Some(
            config
                .builder()
                .map_err(|e| {
                    ApcErrorKind::GeneralError(format!("Could not config reliability pool {e:?}"))
                })?
                .create_timeout(Some(CONNECTION_EXPIRATION.to_std().unwrap()))
                .runtime(deadpool::Runtime::Tokio1)
                .build()
                .map_err(|e| {
                    ApcErrorKind::GeneralError(format!("Could not build reliability pool {e:?}"))
                })?,
        );

        Ok(Self {
            pool,
            db: db.clone(),
            metrics: metrics.clone(),
            retries,
        })
    }

    // Record the message state change to storage.
    pub async fn record(
        &self,
        reliability_id: &Option<String>,
        new: ReliabilityState,
        old: &Option<ReliabilityState>,
        expr: Option<u64>,
    ) -> Result<Option<ReliabilityState>> {
        let Some(id) = reliability_id else {
            return Ok(None);
        };
        if let Some(pool) = &self.pool {
            debug!(
                "🔍 Changing state for {} from {} to {}",
                id,
                old.map(|v| v.to_string())
                    .unwrap_or_else(|| "None".to_owned()),
                new
            );
            match pool.get().await {
                Ok(mut conn) => {
                    self.internal_record(&mut conn, old, new, expr, id)
                        .await
                        .map_err(|e| {
                            warn!("🔍⚠️ Unable to record reliability state: {:?}", e);
                            ApcErrorKind::GeneralError(
                                "Could not record reliability state".to_owned(),
                            )
                        })?;
                }
                Err(e) => warn!("🔍⚠️ Unable to get reliability state pool, {:?}", e),
            };
        };
        // Errors are not fatal, and should not impact message flow, but
        // we should record them somewhere.
        let _ = self.db.log_report(id, new).await.inspect_err(|e| {
            warn!("🔍⚠️ Unable to record reliability state log: {:?}", e);
        });
        Ok(Some(new))
    }

    /// Record the state change in the reliability datastore.
    /// Because of a strange duplication error, we're using three "tables".
    /// The COUNTS table contains the count of messages in each state. It's provided as a quick lookup
    /// for the dashboard query.
    /// The EXPIRY table contains the expiration time for messages. Those are cleaned up by the
    /// `gc` function.
    /// Finally, there's the "table" that is `state.{id}`. This contains the current state of each
    /// message. Ideally, this would use something like `HSETEX`, but that is not available until
    /// Redis 8.0, so for now, we use the `SET` function, which provides an expiration time.
    /// BE SURE TO SET AN EXPIRATION TIME FOR EACH STATE MESSAGE!
    // TODO: Do we need to keep the state in the message? Probably good for sanity reasons.
    pub(crate) async fn internal_record<C: ConnectionLike + AsyncCommands>(
        &self,
        conn: &mut C,
        old: &Option<ReliabilityState>,
        new: ReliabilityState,
        expr: Option<u64>,
        id: &str,
    ) -> Result<()> {
        let state_key = format!("state.{id}");
        /* trace!("🔍 state key: {}", &state_key); */
        // The first state is special, since we need to create the `state_key`.
        if new == ReliabilityState::Received {
            // we can't perform this in a transaction because we can only increment if the set succeeds.
            // Create the new `state.{id}` key if it does not exist, and set the expiration.
            let options = redis::SetOptions::default()
                .with_expiration(redis::SetExpiry::EX(expr.unwrap_or(0)))
                .conditional_set(redis::ExistenceCheck::NX);
            conn.set_options::<_, _, ()>(&state_key, new.to_string(), options)
                .await
                .map_err(|e| {
                    warn!("🔍⚠️ Could not create state key: {:?}", e);
                    ApcErrorKind::GeneralError("Could not create the state key".to_owned())
                })?;
        } else {
            // safety check (yes, there's still a slight chance of a race, but it's small)
            if let Some(old) = old {
                let check_state: String = conn.get(&state_key).await?;
                if check_state != old.to_string() {
                    trace!(
                        "🔍⚠️ Attempting to update state for {} from {} to {}, but current state is different: {:?}",
                        id, old, new, check_state
                    );
                }
                // Sometimes Redis doesn't update the old record. If we have a newer state, presume things
                // are good. Again, there's a case where we could legitimately regress, but hopefully, that
                // is infrequent.
                if old > &new {
                    warn!(
                        "🔍⚠️ State mismatch, ignoring move from: {:?} => {:?}",
                        &old, &new
                    );
                    return Err(ApcErrorKind::GeneralError(
                        "State mismatch during reliability record update".to_owned(),
                    )
                    .into());
                }
            };
        }
        crate::redis_util::transaction(
            conn,
            &[&state_key], // Note: The EXPIRY table changes frequently, and will cause transactions to fail.
            self.retries,
            || ApcErrorKind::GeneralError("Exceeded reliability record retry attempts".to_owned()),
            async |conn, pipe: &mut Pipeline| {
                // Remove the old state from the expiry set, if it exists.
                // There should only be one message at a given state in the `expiry` table.
                // Since we only use that table to track messages that may expire. (We
                // decrement "expired" messages in the `gc` function, so having messages
                // in multiple states may decrement counts incorrectly.))
                if let Some(old) = old {
                    trace!("🔍 ➖: {:?}, {:?}", &old, &id);
                    pipe.hincr(COUNTS, old.to_string(), -1);
                    let old_exp = ExpiryKey {
                        id: id.to_string(),
                        state: old.to_owned(),
                    }
                    .to_string();
                    pipe.zrem(EXPIRY, &old_exp);
                }
                if !new.is_terminal() {
                    // Write the expiration only if the state is non-terminal. Otherwise we run the risk of
                    // messages reporting a false "expired" state even if they were "successful".
                    let new_exp = ExpiryKey {
                        id: id.to_string(),
                        state: new.to_owned(),
                    }
                    .to_string();
                    pipe.zadd(EXPIRY, &new_exp, expr.unwrap_or_default());
                }
                trace!("🔍 ➕ {:?}, {:?}", &new, &id);
                // Bump up the new state count, and set the state key's state if it still exists.
                pipe.hincr(COUNTS, new.to_string(), 1);
                let options = redis::SetOptions::default()
                    .conditional_set(redis::ExistenceCheck::XX)
                    .get(true);
                pipe.set_options(&state_key, new.to_string(), options);
                // `exec_query` returns `RedisResult<()>`.
                // `query_async` returns `RedisResult<Option<redis::Value>, RedisError>`.
                // We really don't care about the returned result here, but transaction
                // retries if we return Ok(None), so we run the exec and return
                // a nonce `Some` value.
                // The turbo-fish is a fallback for edition 2024
                let result = pipe.query_async::<redis::Value>(conn).await?;
                // The last element returned from the command is the result of the pipeline.
                // The last of which should be the result of the `SET` command, which has `GET`
                // set. This should either return the prior value or `Ok` if things worked, else
                // it should return `nil`.
                let rcheck = result
                    .as_sequence()
                    .unwrap_or_default()
                    .last()
                    /*
                    .inspect(|v| {
                        let ss: String = redis::from_redis_value(v)
                            .inspect_err(|e| {
                                warn!(
                                    "🔍⚠️Error occurred while decoding previous state_key: {:?}",
                                    e
                                );
                            })
                            .unwrap_or_default();
                        trace!("🔍 state_key previous value: {}", ss);
                    })
                    // */
                    .unwrap_or(&redis::Value::Nil);
                trace!(
                    "🔍 state_key set {} to {} returned: {:?}",
                    &state_key,
                    new.to_string(),
                    &rcheck
                );
                if new != ReliabilityState::Received && rcheck == &redis::Value::Nil {
                    // If the state_key was not set, then we have a problem.
                    warn!(
                        "🔍⚠️ non-initial set returned nil value: {:?} => {:?} [{:?}]",
                        &old, &new, result
                    );
                }
                /*
                //
                if new != ReliabilityState::Received {
                    if *rcheck != redis::Value::Nil {
                        Ok(Some(redis::Value::Okay))
                    } else {
                        warn!("🔍⚠️ non-initial state_key set returned nil value, retrying transaction")
                        // Something odd went on, retry.
                        Ok(None)
                    }
                } else {
                    // Received will never have a prior value, so will return nil. Sadly,
                    // if this fails, it will also return nil, so presume all is well.
                    Ok(Some(redis::Value::Okay))
                }
                // */
                Ok(Some(redis::Value::Okay))
            },
        )
        .await
        .map_err(|e| {
            warn!("🔍⚠️Error occurred during transaction: {:?}", e);
            e
        })?;
        Ok(())
    }

    /// Perform a garbage collection cycle on a reliability object.
    pub async fn gc(&self) -> Result<()> {
        if let Some(pool) = &self.pool {
            if let Ok(mut conn) = pool.get().await {
                debug!("🔍 performing pre-report garbage collection");
                return self.internal_gc(&mut conn, sec_since_epoch()).await;
            }
        }
        Ok(())
    }

    // Perform the `garbage collection` cycle. This will scan the currently known timestamp
    // indexed entries in redis looking for "expired" data, and then rectify the counts to
    // indicate the final states. This is because many of the storage systems do not provide
    // indicators when data reaches a TTL.

    // A few notes about redis:
    // "pipeline" essentially stitches commands together. Each command executes in turn, but the data store
    // remains open for other operations to occur.
    // `atomic()` wraps things in a transaction, essentially locking the data store while the command executes.
    // Sadly, there's no way to create a true transaction where you read and write in a single operation, so we
    // have to presume some "slop" here.
    pub(crate) async fn internal_gc<C: ConnectionLike + AsyncCommands>(
        &self,
        conn: &mut C,
        expr: u64,
    ) -> Result<()> {
        let result: redis::Value = crate::redis_util::transaction(
            conn,
            &[EXPIRY],
            self.retries,
            || ApcErrorKind::GeneralError("Exceeded gc retry attempts".to_owned()),
            async |conn, pipe| {
                // First, get the list of values that are to be purged.
                let purged: Vec<String> = conn.zrangebyscore(EXPIRY, 0, expr as isize).await?;
                // insta-bail if there's nothing to do.
                if purged.is_empty() {
                    return Ok(Some(redis::Value::Nil));
                }

                // Now purge each of the values by resetting the counts and removing the item from
                // the purge set.
                for key in purged {
                    let Ok(expiry_key) = ExpiryKey::try_from(key.clone()) else {
                        let err = "Invalid key stored in Reliability datastore";
                        error!("🔍🟥 {} [{:?}]", &err, &key);
                        return Err(ApcErrorKind::GeneralError(err.to_owned()));
                    };
                    // Adjust the COUNTS and then remove the record from the list of expired rows.
                    pipe.hincr(COUNTS, expiry_key.state.to_string(), -1);
                    pipe.hincr(COUNTS, ReliabilityState::Expired.to_string(), 1);
                    pipe.zrem(EXPIRY, key);
                }
                Ok(pipe.query_async(conn).await?)
            },
        )
        .await?;
        self.metrics
            .incr_with_tags(MetricName::ReliabilityGc)
            .with_tag(
                "status",
                if result == redis::Value::Nil {
                    "error"
                } else {
                    "success"
                },
            )
            .send();
        Ok(())
    }

    // Return a snapshot of milestone states
    // This will probably not be called directly, but useful for debugging.
    pub async fn report(&self) -> Result<HashMap<String, i32>> {
        if let Some(pool) = &self.pool {
            if let Ok(mut conn) = pool.get().await {
                return Ok(conn.hgetall(COUNTS).await.map_err(|e| {
                    ApcErrorKind::GeneralError(format!("Could not read report {e:?}"))
                })?);
            }
        }
        Ok(HashMap::new())
    }

    pub async fn health_check<'a>(&self) -> Result<&'a str> {
        if let Some(pool) = &self.pool {
            let mut conn = pool.get().await.map_err(|e| {
                ApcErrorKind::GeneralError(format!(
                    "Could not connect to reliability datastore: {e:?}"
                ))
            })?;
            // Add a type here, even though we're tossing the value, in order to prevent the `FromRedisValue` warning.
            conn.ping::<()>().await.map_err(|e| {
                ApcErrorKind::GeneralError(format!("Could not ping reliability datastore: {e:?}"))
            })?;
            Ok("OK")
        } else {
            Ok("OK")
        }
    }
}

const METRIC_NAME: &str = "autopush_reliability";

/// Generate a Prometheus compatible report. Output should follow the
/// [instrumentation](https://prometheus.io/docs/practices/instrumentation/) guidelines.
///
/// In short form, the file should be a plain text output, with each metric on it's own line
/// using the following format:
/// ```text
/// # HELP metric_name Optional description of this metric
/// # TYPE metric_name {required type (gauge|count|histogram|summary)}
/// metric_name{label="label1"} value
/// metric_name{label="label2"} value
/// ```
/// An example which would return counts of messages in given states at the current
/// time would be:
/// ```text
/// # HELP autopush_reliability Counts for messages in given states
/// # TYPE metric_name gauge
/// autopush_reliability{state="recv"} 123
/// autopush_reliability{state="stor"} 123
/// # EOF
/// ```
/// Note that time is not required. A timestamp has been added to the output, but is
/// ignored by Prometheus, and is only provided to ensure that there is no intermediate
/// caching occurring.
///
/// The report endpoint currently is only provided by `autoendpoint`, even though the report
/// is inclusive for all push milestones. This is done for simplicity, both for serving the
/// data and for collection and management of the metrics.
///
pub fn gen_report(values: HashMap<String, i32>) -> Result<String> {
    let mut registry = Registry::default();

    // A "family" is a grouping of metrics.
    // we specify this as the ("label", "label value") which index to a Gauge.
    let family = Family::<Vec<(&str, String)>, Gauge>::default();
    // This creates the top level association of the elements in the family with the metric.
    registry.register(
        METRIC_NAME,
        "Count of messages at given states",
        family.clone(),
    );
    for (milestone, value) in values.into_iter() {
        // Specify the static "state" label name with the given milestone, and add the
        // value as the gauge value.
        family
            .get_or_create(&vec![("state", milestone)])
            .set(value.into());
    }

    // Return the formatted string that Prometheus will eventually read.
    let mut encoded = String::new();
    encode(&mut encoded, &registry).map_err(|e| {
        ApcErrorKind::GeneralError(format!("Could not generate Reliability report {e:?}"))
    })?;
    Ok(encoded)
}

/// Handle the `/metrics` request by returning a Prometheus compatible report.
pub async fn report_handler(reliability: &Arc<PushReliability>) -> Result<HttpResponse> {
    reliability.gc().await?;
    let report = gen_report(reliability.report().await?)?;
    Ok(HttpResponse::Ok()
        .content_type("application/openmetrics-text; version=1.0.0; charset=utf-8")
        .body(report))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::str::FromStr;

    use super::*;
    use redis_test::{MockCmd, MockRedisConnection};
    use uuid::Uuid;

    #[test]
    fn test_report() {
        // create a nonce report
        let mut report: HashMap<String, i32> = HashMap::new();
        let recv = ReliabilityState::Received.to_string();
        let trns = ReliabilityState::Transmitted.to_string();
        report.insert(recv.clone(), 111);
        report.insert(ReliabilityState::Stored.to_string(), 222);
        report.insert(ReliabilityState::Retrieved.to_string(), 333);
        report.insert(trns.clone(), 444);

        let generated = gen_report(report).unwrap();
        // We don't really care if the `Created` or `HELP` lines are included
        assert!(generated.contains(&format!("# TYPE {METRIC_NAME}")));
        // sample the first and last values.
        assert!(generated.contains(&format!("{METRIC_NAME}{{state=\"{recv}\"}} 111")));
        assert!(generated.contains(&format!("{METRIC_NAME}{{state=\"{trns}\"}} 444")));
    }

    #[test]
    fn state_ser() {
        assert_eq!(
            ReliabilityState::from_str("delivered").unwrap(),
            ReliabilityState::Delivered
        );
        assert_eq!(
            ReliabilityState::from_str("int_accepted").unwrap(),
            ReliabilityState::IntAccepted
        );
        assert_eq!(
            serde_json::from_str::<ReliabilityState>(r#""int_accepted""#).unwrap(),
            ReliabilityState::IntAccepted
        );

        assert_eq!(ReliabilityState::IntAccepted.to_string(), "int_accepted");
        assert_eq!(
            serde_json::to_string(&ReliabilityState::IntAccepted).unwrap(),
            r#""int_accepted""#
        );
    }

    #[actix_rt::test]
    async fn test_push_reliability_report() -> Result<()> {
        let mut db = crate::db::mock::MockDbClient::new();

        // Build the state.
        let test_id = format!("TEST_VALUE_{}", Uuid::new_v4());
        // Remember, we shouldn't just arbitrarily create mid-state values, so
        // let's start from the beginning
        let new = ReliabilityState::Received;
        let old = None;
        let expr = 1;

        let exp_key = ExpiryKey {
            id: test_id.clone(),
            state: new,
        }
        .to_string();

        let mut pipeline = redis::Pipeline::new();
        // We're adding an element so we have something to record.
        pipeline
            .cmd("MULTI")
            // No old state, so no decrement.
            // "received" is not terminal
            .cmd("ZADD")
            .arg(EXPIRY)
            .arg(expr)
            .arg(&exp_key)
            // Increment the count for the new state.
            .cmd("HINCRBY")
            .arg(COUNTS)
            .arg(new.to_string())
            .arg(1)
            // Create/update the state.holder value.
            .cmd("SET")
            .arg(format!("state.{test_id}"))
            .arg(new.to_string())
            .arg("XX")
            .arg("GET")
            // Run the transaction
            .cmd("EXEC");
        let mut conn = MockRedisConnection::new(vec![
            MockCmd::new(
                redis::cmd("SET")
                    .arg(format!("state.{test_id}"))
                    .arg(new.to_string())
                    .arg("NX")
                    .arg("EX")
                    .arg(expr),
                Ok(redis::Value::Okay),
            ),
            MockCmd::new(
                redis::cmd("WATCH").arg(format!("state.{test_id}")),
                Ok(redis::Value::Okay),
            ),
            MockCmd::new(
                pipeline,
                Ok(redis::Value::Array(vec![
                    redis::Value::Okay,                              // MULTI
                    redis::Value::SimpleString("QUEUED".to_owned()), // ZADD
                    redis::Value::SimpleString("QUEUED".to_owned()), // HINCRBY
                    redis::Value::SimpleString("QUEUED".to_owned()), // SET
                    // Return the transaction results.
                    redis::Value::Array(vec![
                        redis::Value::Int(1), // 0 -> 1
                        redis::Value::Int(1),
                        // there's no prior value, so this will return Nil
                        redis::Value::Nil,
                    ]),
                ])),
            ),
            MockCmd::new(redis::cmd("UNWATCH"), Ok(redis::Value::Okay)),
        ]);

        let metrics = Arc::new(StatsdClient::builder("", cadence::NopMetricSink).build());

        let int_test_id = test_id.clone();
        db.expect_log_report()
            .times(1)
            .withf(move |id, state| id == int_test_id && state == &new)
            .return_once(|_, _| Ok(()));
        // test the main report function (note, this does not test redis)
        let db_box = Box::new(Arc::new(db));
        let pr = PushReliability::new(
            &None,
            db_box.clone(),
            &metrics,
            crate::redis_util::MAX_TRANSACTION_LOOP,
        )
        .unwrap();
        // `.record()` uses a pool, so we can't pass the moc connection directly.
        // Instead, we're going to essentially recreate what `.record()` does calling
        // the `.internal_record()` function, followed by the database `.record()`.
        //pr.record(&Some(test_id.clone()), new, &None, Some(expr))
        //    .await?;

        // and mock the redis call.
        pr.internal_record(&mut conn, &old, new, Some(expr), &test_id)
            .await
            .inspect_err(|e| {
                dbg!("🔍⚠️ Unable to record reliability state: {:?}", e);
            })?;

        db_box
            .log_report(&test_id, new)
            .await
            .inspect_err(|e| {
                warn!("🔍⚠️ Unable to record reliability state log: {:?}", e);
            })
            .map_err(|e| ApcErrorKind::GeneralError(e.to_string()))?;

        Ok(())
    }

    //*
    #[actix_rt::test]
    async fn test_push_reliability_record() -> Result<()> {
        let db = crate::db::mock::MockDbClient::new();
        let test_id = format!("TEST_VALUE_{}", Uuid::new_v4());
        let new = ReliabilityState::Stored;
        let old = ReliabilityState::Received;
        let expr = 1;

        let metrics = Arc::new(StatsdClient::builder("", cadence::NopMetricSink).build());
        let new_key = ExpiryKey {
            id: test_id.clone(),
            state: new,
        }
        .to_string();
        let old_key = ExpiryKey {
            id: test_id.clone(),
            state: old,
        }
        .to_string();
        let mut mock_pipe = redis::Pipeline::new();
        mock_pipe
            .cmd("MULTI")
            .ignore()
            .cmd("HINCRBY")
            .arg(COUNTS)
            .arg(new.to_string())
            .arg(1)
            .ignore()
            .cmd("HINCRBY")
            .arg(COUNTS)
            .arg(old.to_string())
            .arg(-1)
            .ignore()
            .cmd("ZREM")
            .arg(EXPIRY)
            .arg(old_key)
            .ignore()
            .cmd("ZADD")
            .arg(EXPIRY)
            .arg(1)
            .arg(new_key)
            .ignore()
            .cmd("EXEC")
            .ignore();

        let mut conn = MockRedisConnection::new(vec![
            // Create the new state
            MockCmd::new(
                redis::cmd("SET")
                    .arg(format!("state.{test_id}"))
                    .arg(new.to_string())
                    .arg("NX")
                    .arg("EX")
                    .arg(expr),
                Ok(redis::Value::Okay),
            ),
            // Increment the count for new
            MockCmd::new(
                redis::cmd("HINCRBY")
                    .arg(COUNTS)
                    .arg(new.to_string())
                    .arg(1),
                Ok(redis::Value::Okay),
            ),
            MockCmd::new(
                redis::cmd("WATCH").arg(COUNTS).arg(EXPIRY),
                Ok(redis::Value::Okay),
            ),
            // NOTE: Technically, since we `.ignore()` these, we could just have a
            // Vec containing just `Okay`. I'm being a bit pedantic here because I know
            // that this will come back to haunt me if I'm not, and because figuring out
            // the proper response for this was annoying.
            MockCmd::new(
                mock_pipe,
                Ok(redis::Value::Array(vec![
                    redis::Value::Okay,
                    // Match the number of commands that are being held for processing
                    redis::Value::SimpleString("QUEUED".to_owned()),
                    redis::Value::SimpleString("QUEUED".to_owned()),
                    redis::Value::SimpleString("QUEUED".to_owned()),
                    redis::Value::SimpleString("QUEUED".to_owned()),
                    // the exec has been called, return an array containing the results.
                    redis::Value::Array(vec![
                        redis::Value::Okay,
                        redis::Value::Okay,
                        redis::Value::Okay,
                        redis::Value::Okay,
                    ]),
                ])),
            ),
            // If the transaction fails, this should return a redis::Value::Nil
            MockCmd::new(redis::cmd("UNWATCH"), Ok(redis::Value::Okay)),
        ]);

        // test the main report function (note, this does not test redis)
        let pr = PushReliability::new(
            &None,
            Box::new(Arc::new(db)),
            &metrics,
            crate::redis_util::MAX_TRANSACTION_LOOP,
        )
        .unwrap();
        let _ = pr
            .internal_record(&mut conn, &Some(old), new, Some(expr), &test_id)
            .await;

        Ok(())
    }
    // */
    //*
    #[actix_rt::test]
    async fn test_push_reliability_full() -> Result<()> {
        let db = crate::db::mock::MockDbClient::new();
        let test_id = format!("TEST_VALUE_{}", Uuid::new_v4());
        let new = ReliabilityState::Received;
        let stored = ReliabilityState::Stored;
        let expr = 1;

        let metrics = Arc::new(StatsdClient::builder("", cadence::NopMetricSink).build());
        let new_key = ExpiryKey {
            id: test_id.clone(),
            state: new,
        }
        .to_string();
        let stored_key = ExpiryKey {
            id: test_id.clone(),
            state: stored,
        }
        .to_string();

        let state_key = format!("state.{test_id}");

        let mut mock_pipe = redis::Pipeline::new();
        mock_pipe
            .cmd("MULTI")
            .ignore()
            .cmd("HINCRBY")
            .arg(COUNTS)
            .arg(stored.to_string())
            .arg(-1)
            .ignore()
            .cmd("ZREM")
            .arg(EXPIRY)
            .arg(stored_key.to_string())
            .ignore()
            .cmd("ZADD")
            .arg(EXPIRY)
            .arg(1)
            .arg(new_key)
            .ignore()
            .cmd("HINCRBY")
            .arg(COUNTS)
            .arg(new.to_string())
            .arg(1)
            .ignore()
            .cmd("SET")
            .arg(&state_key)
            .arg(new.to_string())
            .arg("XX")
            .arg("GET")
            .cmd("EXEC")
            .ignore();

        let mut conn = MockRedisConnection::new(vec![
            MockCmd::new(
                // Create the new state
                redis::cmd("SET")
                    .arg(format!("state.{test_id}"))
                    .arg(new.to_string())
                    .arg("NX")
                    .arg("EX")
                    .arg(expr),
                Ok(redis::Value::Okay),
            ),
            // increment the count for new
            MockCmd::new(
                redis::cmd("HINCRBY")
                    .arg(COUNTS)
                    .arg(new.to_string())
                    .arg(1),
                Ok(redis::Value::Okay),
            ),
            // begin the transaction
            MockCmd::new(
                redis::cmd("WATCH").arg(&state_key).arg(EXPIRY),
                Ok(redis::Value::Okay),
            ),
            // NOTE: Technically, since we `.ignore()` these, we could just have a
            // Vec containing just `Okay`. I'm being a bit pedantic here because I know
            // that this will come back to haunt me if I'm not, and because figuring out
            // the proper response for this was annoying.
            MockCmd::new(
                mock_pipe,
                Ok(redis::Value::Array(vec![
                    redis::Value::Okay,
                    // Match the number of commands that are being held for processing
                    redis::Value::SimpleString("QUEUED".to_owned()),
                    redis::Value::SimpleString("QUEUED".to_owned()),
                    redis::Value::SimpleString("QUEUED".to_owned()),
                    redis::Value::SimpleString("QUEUED".to_owned()),
                    // the exec has been called, return an array containing the results.
                    redis::Value::Array(vec![
                        redis::Value::Okay,
                        redis::Value::Okay,
                        redis::Value::Okay,
                        redis::Value::Okay,
                    ]),
                ])),
            ),
            // If the transaction fails, this should return a redis::Value::Nil
            MockCmd::new(redis::cmd("UNWATCH"), Ok(redis::Value::Okay)),
        ]);

        // test the main report function (note, this does not test redis)
        let pr = PushReliability::new(
            &None,
            Box::new(Arc::new(db)),
            &metrics,
            crate::redis_util::MAX_TRANSACTION_LOOP,
        )
        .unwrap();
        let _ = pr
            .internal_record(&mut conn, &Some(stored), new, Some(expr), &test_id)
            .await;

        Ok(())
    }
    // */
    #[actix_rt::test]
    async fn test_push_reliability_gc() -> Result<()> {
        let db = crate::db::mock::MockDbClient::new();
        let test_id = format!("TEST_VALUE_{}", Uuid::new_v4());
        let new = ReliabilityState::Accepted;
        let key = format!("{}#{}", &test_id, &new);
        let expr = 1;
        let metrics = Arc::new(StatsdClient::builder("", cadence::NopMetricSink).build());

        let response: redis::Value = redis::Value::Array(vec![redis::Value::SimpleString(
            ExpiryKey {
                id: test_id.clone(),
                state: new,
            }
            .to_string(),
        )]);

        // Construct the Pipeline.
        // A redis "pipeline" is a set of instructions that are executed in order. These
        // are not truly atomic, as other actions can interrupt a pipeline, but they
        // are guaranteed to happen in sequence. Transactions essentially check that the
        // WATCH key is not altered before the pipeline is executed.
        let mut mock_pipe = redis::Pipeline::new();
        mock_pipe
            .cmd("MULTI")
            .ignore()
            .cmd("HINCRBY")
            .arg(COUNTS)
            .arg(new.to_string())
            .arg(-1)
            .ignore()
            .cmd("HINCRBY")
            .arg(COUNTS)
            .arg(ReliabilityState::Expired.to_string())
            .arg(1)
            .ignore()
            .cmd("ZREM")
            .arg(EXPIRY)
            .arg(key)
            .ignore()
            .cmd("EXEC")
            .ignore();

        // This mocks the "conn"ection, so all commands sent to the connection need
        // to be separated out. This includes the transaction "WATCH" and "UNWATCH".
        // The "meat" of the functions are handled via a Pipeline, which conjoins the
        // commands into one group (see above). Pipelines return an array of
        // results, one entry for each pipeline command.
        // In other words, the `internal_gc` commands are probably in the pipeline,
        // all the others are probably in the conn.
        let mut conn = MockRedisConnection::new(vec![
            MockCmd::new(redis::cmd("WATCH").arg(EXPIRY), Ok(redis::Value::Okay)),
            MockCmd::new(
                redis::cmd("ZRANGEBYSCORE").arg(EXPIRY).arg(0).arg(expr),
                Ok(response),
            ),
            // NOTE: Technically, since we `.ignore()` these, we could just have a
            // Vec containing just `Okay`. I'm being a bit pedantic here because I know
            // that this will come back to haunt me if I'm not, and because figuring out
            // the proper response for this was annoying.
            MockCmd::new(
                mock_pipe,
                Ok(redis::Value::Array(vec![
                    redis::Value::Okay,
                    // Match the number of commands that are being held for processing
                    redis::Value::SimpleString("QUEUED".to_owned()),
                    redis::Value::SimpleString("QUEUED".to_owned()),
                    redis::Value::SimpleString("QUEUED".to_owned()),
                    // the exec has been called, return an array containing the results.
                    redis::Value::Array(vec![
                        redis::Value::Okay,
                        redis::Value::Okay,
                        redis::Value::Okay,
                    ]),
                ])),
            ),
            // If the transaction fails, this should return a redis::Value::Nil
            MockCmd::new(redis::cmd("UNWATCH"), Ok(redis::Value::Okay)),
        ]);

        // test the main report function (note, this does not test redis)
        let pr = PushReliability::new(
            &None,
            Box::new(Arc::new(db)),
            &metrics,
            crate::redis_util::MAX_TRANSACTION_LOOP,
        )
        .unwrap();
        // functionally a no-op, but it does exercise lines.
        pr.gc().await?;

        // and mock the redis call.
        pr.internal_gc(&mut conn, 1).await?;

        Ok(())
    }
}
