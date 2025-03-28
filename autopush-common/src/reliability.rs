/// Push Reliability Recorder
///
/// This allows us to track messages from select, known parties (currently, just
/// mozilla generated and consumed) so that we can identify potential trouble spots
/// and where messages expire early. Message expiration can lead to message loss
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use actix_web::HttpResponse;
use prometheus_client::{
    encoding::text::encode, metrics::family::Family, metrics::gauge::Gauge, registry::Registry,
};
use redis::{Commands, ConnectionLike};

use crate::db::client::DbClient;
use crate::errors::{ApcErrorKind, Result};
use crate::util::timing::sec_since_epoch;

// Redis Keys
pub const COUNTS: &str = "state_counts";
pub const EXPIRY: &str = "expiry";

const CONNECTION_EXPIRATION: Duration = Duration::from_secs(10);

/// The various states that a message may transit on the way from reception to delivery.
// Note: "Message" in this context refers to the Subscription Update.
// TODO: Differentiate between "transmitted via webpush" and "transmitted via bridge"?
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Deserialize,
    serde::Serialize,
    strum::Display,
    strum::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ReliabilityState {
    Received,        // Message was received by the Push Server
    Stored,          // Message was stored because it could not be delivered immediately
    Retrieved,       // Message was taken from storage for delivery
    IntTransmitted,  // Message was handed off between autoendpoint and autoconnect
    IntAccepted,     // Message was accepted by autoconnect from autopendpoint
    Transmitted,     // Message was handed off for delivery to the UA
    Accepted,        // Message was accepted for delivery by the UA
    Delivered,       // Message was provided to the WebApp recipient by the UA
    DecryptionError, // Message was provided to the UA and it reported a decryption error
    NotDelivered,    // Message was provided to the UA and it reported a not delivered error
    Expired,         // Message expired naturally (e.g. TTL=0)
    Errored,         // Message resulted in an Error state and failed to be delivered.
}

#[derive(Clone)]
pub struct PushReliability {
    client: Option<Arc<redis::Client>>,
    db: Box<dyn DbClient>,
}

impl PushReliability {
    // Do the magic to make a report instance, whatever that will be.
    pub fn new(reliability_dsn: &Option<String>, db: Box<dyn DbClient>) -> Result<Self> {
        if reliability_dsn.is_none() {
            debug!("🔍 No reliability DSN declared.");
            return Ok(Self {
                client: None,
                db: db.clone(),
            });
        };

        let client = if let Some(dsn) = reliability_dsn {
            let redis_client = redis::Client::open(dsn.clone()).map_err(|e| {
                ApcErrorKind::GeneralError(format!("Could not connect to redis server: {:?}", e))
            })?;
            //Verify that we can connect to the server.
            match redis_client.get_connection_with_timeout(CONNECTION_EXPIRATION) {
                Ok(_) => {
                    info!("🔍 Connected to reliability server at {dsn}");
                    Some(Arc::new(redis_client))
                }
                Err(e) => {
                    // This is a "soft" error. It does impact whether or not reliability will work but it's
                    // not something that should prevent the server from running.
                    error!("🔍 Invalid reliability DNS specified: {e} for DSN: {dsn}.");
                    None
                }
            }
        } else {
            trace!("🔍 Empty Redis DSN?");
            None
        };

        Ok(Self {
            client,
            db: db.clone(),
        })
    }

    // Record the record state change to storage.
    pub async fn record(
        &self,
        reliability_id: &Option<String>,
        new: ReliabilityState,
        old: &Option<ReliabilityState>,
        expr: Option<u64>,
    ) -> Option<ReliabilityState> {
        let Some(id) = reliability_id else {
            return None;
        };
        if let Some(client) = &self.client {
            debug!(
                "🔍 {} from {} to {}",
                id,
                old.map(|v| v.to_string())
                    .unwrap_or_else(|| "None".to_owned()),
                new
            );
            match client.get_connection_with_timeout(CONNECTION_EXPIRATION) {
                Ok(mut conn) => self.internal_record(&mut conn, old, new, expr, id).await,
                Err(e) => warn!("🔍⚠️ Unable to record reliability state: {:?}", e),
            };
        };
        // Errors are not fatal, and should not impact message flow, but
        // we should record them somewhere.
        let _ = self.db.log_report(id, new).await.inspect_err(|e| {
            warn!("🔍⚠️ Unable to record reliability state: {:?}", e);
        });
        Some(new)
    }

    pub(crate) async fn internal_record<C: ConnectionLike>(
        &self,
        conn: &mut C,
        old: &Option<ReliabilityState>,
        new: ReliabilityState,
        expr: Option<u64>,
        id: &str,
    ) {
        let mut pipeline = redis::Pipeline::new();
        let pipeline = pipeline.hincr(COUNTS, new.to_string(), 1);
        let pipeline = if let Some(old) = old {
            pipeline
                .hincr(COUNTS, old.to_string(), -1)
                .zrem(EXPIRY, format!("{}#{}", &old, id))
        } else {
            pipeline
        };
        // Errors are not fatal, and should not impact message flow, but
        // we should record them somewhere.
        let cc = pipeline
            .zadd(EXPIRY, format!("{}#{}", new, id), expr.unwrap_or_default())
            .exec(conn)
            .inspect_err(|e| {
                warn!("🔍 Failed to write to storage: {:?}", e);
            });
        trace!("🔍 internal record result: {:?}", cc);
    }

    /// Perform a garbage collection cycle on a reliability object.
    pub async fn gc(&self) -> Result<()> {
        if let Some(client) = &self.client {
            debug!("🔍 performing pre-report garbage collection");
            if let Ok(mut conn) = client.get_connection_with_timeout(CONNECTION_EXPIRATION) {
                return self.internal_gc(&mut conn, sec_since_epoch()).await;
            }
        }
        Ok(())
    }

    /// Perform the `garbage collection` cycle. This will scan the currently known timestamp
    /// indexed entries in redis looking for "expired" data, and then rectify the counts to
    /// indicate the final states. This is because many of the storage systems do not provide
    /// indicators when data reaches a TTL.
    pub(crate) async fn internal_gc<C: ConnectionLike>(
        &self,
        conn: &mut C,
        expr: u64,
    ) -> Result<()> {
        let purged: Vec<String> = conn.zrangebyscore(EXPIRY, 0, expr as isize)?;
        let mut pipeline = redis::Pipeline::new();
        for key in purged {
            let Some((state, _id)) = key.split_once('#') else {
                let err = "Invalid key stored in Reliability datastore";
                error!("🔍🟥 {} [{:?}]", &err, &key);
                return Err(ApcErrorKind::GeneralError(err.to_owned()).into());
            };
            pipeline.hincr(COUNTS, state, -1);
            pipeline.zrem(EXPIRY, key);
        }
        pipeline.exec(conn)?;
        Ok(())
    }

    // Return a snapshot of milestone states
    // This will probably not be called directly, but useful for debugging.
    pub async fn report(&self) -> Result<HashMap<String, i32>> {
        if let Some(client) = &self.client {
            if let Ok(mut conn) = client.get_connection() {
                return Ok(conn.hgetall(COUNTS).map_err(|e| {
                    ApcErrorKind::GeneralError(format!("Could not read report {:?}", e))
                })?);
            }
        }
        Ok(HashMap::new())
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
        ApcErrorKind::GeneralError(format!("Could not generate Reliability report {:?}", e))
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
        let acpt = ReliabilityState::Accepted.to_string();
        let trns = ReliabilityState::Transmitted.to_string();
        report.insert(acpt.clone(), 111);
        report.insert(ReliabilityState::Stored.to_string(), 222);
        report.insert(ReliabilityState::Retrieved.to_string(), 333);
        report.insert(trns.clone(), 444);

        let generated = gen_report(report).unwrap();
        // We don't really care if the `Created` or `HELP` lines are included
        assert!(generated.contains(&format!("# TYPE {METRIC_NAME}")));
        // sample the first and last values.
        assert!(generated.contains(&format!("{METRIC_NAME}{{state=\"{acpt}\"}} 111")));
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
        let test_id = format!("TEST_VALUE_{}", Uuid::new_v4());
        let new = ReliabilityState::Accepted;
        let old = None;
        let expr = 1;

        let mut conn = MockRedisConnection::new(vec![MockCmd::new(
            redis::cmd("ZADD")
                .arg(EXPIRY)
                .arg(format!("{}#{}", test_id, new.clone()))
                .arg(expr),
            Ok(""),
        )]);

        let int_test_id = test_id.clone();
        db.expect_log_report()
            .times(1)
            .withf(move |id, state| id == int_test_id && state == &ReliabilityState::Accepted)
            .return_once(|_, _| Ok(()));
        // test the main report function (note, this does not test redis)
        let pr = PushReliability::new(&None, Box::new(Arc::new(db))).unwrap();
        pr.record(&Some(test_id.clone()), new, &None, Some(expr))
            .await;

        // and mock the redis call.
        pr.internal_record(&mut conn, &old, new, Some(expr), &test_id)
            .await;

        Ok(())
    }

    #[actix_rt::test]
    async fn test_push_reliability_gc() -> Result<()> {
        let db = crate::db::mock::MockDbClient::new();
        let test_id = format!("TEST_VALUE_{}", Uuid::new_v4());
        let new = ReliabilityState::Accepted;
        let key = format!("{}#{}", &test_id, &new);
        let expr = 1;

        let response: redis::Value = redis::Value::Array(vec![redis::Value::SimpleString(
            format!("{}#{}", test_id, new.clone()),
        )]);

        let mut mock_pipe = redis::Pipeline::new();
        mock_pipe
            .cmd("HINCRBY")
            .arg(COUNTS)
            .arg(test_id.to_string())
            .arg(-1)
            .ignore()
            .cmd("ZREM")
            .arg(EXPIRY)
            .arg(key)
            .ignore();
        let mut conn = MockRedisConnection::new(vec![
            MockCmd::new(
                redis::cmd("ZRANGEBYSCORE").arg(EXPIRY).arg(0).arg(expr),
                Ok(response),
            ),
            MockCmd::new(mock_pipe, Ok("Okay")),
        ]);

        // test the main report function (note, this does not test redis)
        let pr = PushReliability::new(&None, Box::new(Arc::new(db))).unwrap();
        // functionally a no-op, but it does exercise lines.
        pr.gc().await?;

        // and mock the redis call.
        pr.internal_gc(&mut conn, 1).await?;

        Ok(())
    }
}
