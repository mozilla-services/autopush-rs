use std::collections::HashSet;
use std::env;
use std::sync::Arc;
use uuid::Uuid;

use cadence::StatsdClient;
use futures::{future, Future};
use futures_backoff::retry_if;

use crate::db::DbSocketClient;
use crate::errors::{ApiError, ApiErrorKind, ApiResult};
use crate::notification::Notification;
use crate::util::timing::sec_since_epoch;

use commands::{
    retryable_batchwriteitem_error, retryable_delete_error, retryable_putitem_error,
    retryable_updateitem_error, FetchMessageResponse,
};

pub use rusoto_core::{HttpClient, Region};
pub use rusoto_credential::StaticProvider;
pub use rusoto_dynamodb::{
    AttributeValue, BatchWriteItemInput, DeleteItemInput, DynamoDb, DynamoDbClient, PutItemInput,
    PutRequest, UpdateItemInput, UpdateItemOutput, WriteRequest,
};

use super::{CheckStorageResponse, HelloResponse, RegisterResponse, MAX_EXPIRY};
use super::{NotificationRecord, UserRecord};

#[macro_use]
pub mod macros;
pub mod commands;

#[derive(Clone)]
pub struct DynamoStorage {
    db_client: DynamoDbClient,
    metrics: Arc<StatsdClient>,
    router_table_name: String,
    pub message_table_names: Vec<String>,
    pub current_message_month: String,
}

#[async_trait::async_trait]
impl DbSocketClient for DynamoStorage {
    async fn hello(
        &self,
        connected_at: u64,
        uaid: Option<&Uuid>,
        router_url: &str,
        defer_registration: bool,
    ) -> ApiResult<HelloResponse> {
        trace!(
            "### uaid {:?}, defer_registration: {:?}",
            &uaid,
            &defer_registration
        );
        let response: (HelloResponse, Option<UserRecord>) = if let Some(uaid) = uaid {
            commands::lookup_user(
                self.db_client.clone(),
                self.metrics.clone(),
                uaid,
                connected_at,
                router_url,
                &self.router_table_name,
                &self.message_table_names,
                &self.current_message_month,
            )
            .await?
        } else {
            (
                HelloResponse {
                    message_month: self.current_message_month.clone(),
                    connected_at,
                    ..Default::default()
                },
                None,
            )
        };
        let ddb = self.db_client.clone();
        let router_url = router_url.to_string();
        let router_table_name = self.router_table_name.clone();
        let connected_at = connected_at;

        let (mut hello_response, user_opt) = response;
        trace!(
            "### Hello Response: {:?}, {:?}",
            hello_response.uaid,
            user_opt
        );
        let hello_message_month = hello_response.message_month.clone();
        let user = user_opt.unwrap_or_else(|| UserRecord {
            current_month: Some(hello_message_month),
            node_id: Some(router_url),
            connected_at,
            ..Default::default()
        });
        let uaid = user.uaid;
        trace!("### UAID = {:?}", &uaid);
        let mut err_response = hello_response.clone();
        err_response.connected_at = connected_at;
        if !defer_registration {
            return match commands::register_user(ddb, &user, &router_table_name).await {
                Ok(result) => {
                    debug!("Success adding user, item output: {:?}", result);
                    hello_response.uaid = Some(uaid);
                    Ok(hello_response)
                }
                Err(e) => {
                    debug!("Error registering user: {:?}", e);
                    Ok(err_response)
                }
            };
        } else {
            debug!("Deferring user registration {:?}", &uaid);
            hello_response.uaid = Some(uaid);
            hello_response.deferred_user_registration = Some(user);
            return Ok(hello_response);
        }
        /*
        if !defer_registration {
            future::Either::A(
                match commands::register_user(ddb, &user, &router_table_name).await {
                    Ok(result) => {
                        debug!("Success adding user, item output: {:?}", result);
                        hello_response.uaid = Some(uaid);
                        Ok(hello_response)
                    }
                    Err(e) => {
                        debug!("Error registering user: {:?}", e);
                        Ok(err_response)
                    },
                }
            )
        } else {
            debug!("Deferring user registration {:?}", &uaid);
            hello_response.uaid = Some(uaid);
            hello_response.deferred_user_registration = Some(user);
            future::Either::B(Box::new(future::ok(hello_response)))
        }
        */
    }

    async fn register(
        &self,
        uaid: &Uuid,
        channel_id: &Uuid,
        message_month: &str,
        endpoint: &str,
        register_user: Option<&UserRecord>,
    ) -> ApiResult<RegisterResponse> {
        let ddb = self.db_client.clone();
        let mut chids = HashSet::new();
        let endpoint = endpoint.to_owned();
        chids.insert(channel_id.to_hyphenated().to_string());

        if let Some(user) = register_user {
            trace!(
                "### Endpoint Request: User not yet registered... {:?}",
                &user.uaid
            );
            trace!("message month: {:?}", &message_month);
            let uaid2 = *uaid;
            let message_month2 = message_month.to_owned();
            commands::register_user(ddb.clone(), user, &self.router_table_name).await?;
            trace!("### Saving channels: {:#?}", chids);
            match commands::save_channels(ddb, &uaid2, chids, &message_month2).await {
                Ok(_) => {
                    trace!("### sending endpoint: {}", endpoint);
                    return Ok(RegisterResponse::Success { endpoint });
                }
                Err(r) => {
                    trace!("--- failed to register channel. {:?}", r);
                    return Ok(RegisterResponse::Error {
                        status: 503,
                        error_msg: "Failed to register channel".to_string(),
                    });
                }
            }
        };
        trace!("### Continuing...");
        return match commands::save_channels(ddb, uaid, chids, message_month).await {
            Ok(_) => Ok(RegisterResponse::Success { endpoint }),
            Err(_) => Ok(RegisterResponse::Error {
                status: 503,
                error_msg: "Failed to register channel".to_string(),
            }),
        };
    }

    async fn drop_uaid(&self, uaid: &Uuid) -> ApiResult<()> {
        commands::drop_user(self.db_client.clone(), uaid, &self.router_table_name).await?;
        Ok(())
    }

    async fn unregister(
        &self,
        uaid: &Uuid,
        channel_id: &Uuid,
        message_month: &str,
    ) -> ApiResult<bool> {
        match commands::unregister_channel_id(
            self.db_client.clone(),
            uaid,
            channel_id,
            message_month,
        )
        .await
        {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    /// Migrate a user to a new month table

    async fn migrate_user(&self, uaid: &Uuid, message_month: &str) -> ApiResult<()> {
        let uaid = *uaid;
        let ddb = self.db_client.clone();
        let ddb2 = self.db_client.clone();
        let cur_month = self.current_message_month.to_string();
        let cur_month2 = cur_month.clone();
        let router_table_name = self.router_table_name.clone();

        let channels = commands::all_channels(self.db_client.clone(), &uaid, message_month).await?;
        if !channels.is_empty() {
            commands::save_channels(ddb, &uaid, channels, &cur_month).await?;
            commands::update_user_message_month(ddb2, &uaid, &router_table_name, &cur_month2)
                .await?;
        }
        Ok(())
    }

    /// Store a single message
    async fn store_message(
        &self,
        uaid: &Uuid,
        message_month: String,
        message: Notification,
    ) -> ApiResult<()> {
        let ddb = self.db_client.clone();
        let put_item = PutItemInput {
            item: serde_dynamodb::to_hashmap(&NotificationRecord::from_notif(uaid, message))
                .unwrap(),
            table_name: message_month,
            ..Default::default()
        };

        ddb.put_item(put_item.clone())
            .await
            .map_err(|e| ApiErrorKind::DatabaseError(e.to_string()))?;
        Ok(())
    }

    /// Store a batch of messages when shutting down
    async fn store_messages(
        &self,
        uaid: &Uuid,
        message_month: &str,
        messages: Vec<Notification>,
    ) -> ApiResult<()> {
        let ddb = self.db_client.clone();
        let put_items: Vec<WriteRequest> = messages
            .into_iter()
            .filter_map(|n| {
                serde_dynamodb::to_hashmap(&NotificationRecord::from_notif(uaid, n))
                    .ok()
                    .map(|hm| WriteRequest {
                        put_request: Some(PutRequest { item: hm }),
                        delete_request: None,
                    })
            })
            .collect();
        let batch_input = BatchWriteItemInput {
            request_items: hashmap! { message_month.to_string() => put_items },
            ..Default::default()
        };

        ddb.batch_write_item(batch_input.clone())
            .await
            .map_err(|err| {
                debug!("Error saving notification {:?}", err);
                ApiErrorKind::DatabaseError(err.to_string())
            })?;
        Ok(())
    }

    /// Delete a given notification from the database
    ///
    /// No checks are done to see that this message came from the database or has
    /// sufficient properties for a delete as that is expected to have been done
    /// before this is called.
    async fn delete_message(
        &self,
        table_name: &str,
        uaid: &Uuid,
        notif: &Notification,
    ) -> ApiResult<()> {
        let ddb = self.db_client.clone();
        let delete_input = DeleteItemInput {
            table_name: table_name.to_string(),
            key: ddb_item! {
               uaid: s => uaid.to_simple().to_string(),
               chidmessageid: s => notif.sort_key()
            },
            ..Default::default()
        };

        ddb.delete_item(delete_input.clone())
            .await
            .map_err(|err| ApiErrorKind::DatabaseError(err.to_string()))?;
        Ok(())
    }

    async fn check_storage(
        &self,
        table_name: &str,
        uaid: &Uuid,
        include_topic: bool,
        timestamp: Option<u64>,
    ) -> ApiResult<CheckStorageResponse> {
        let response: FetchMessageResponse = if include_topic {
            commands::fetch_messages(
                self.db_client.clone(),
                self.metrics.clone(),
                table_name,
                uaid,
                11,
            )
            .await?
        } else {
            Default::default()
        };
        let uaid = *uaid;
        let table_name = table_name.to_string();
        let ddb = self.db_client.clone();
        let metrics = self.metrics.clone();

        // Return now from this future if we have messages
        if !response.messages.is_empty() {
            debug!("Topic message returns: {:?}", response.messages);
            return Ok(CheckStorageResponse {
                include_topic: true,
                messages: response.messages,
                timestamp: response.timestamp,
            });
        }
        // Use the timestamp returned by the topic query if we were looking at the topics
        let timestamp = if include_topic {
            response.timestamp
        } else {
            timestamp
        };
        let next_query = {
            if response.messages.is_empty() || response.timestamp.is_some() {
                commands::fetch_timestamp_messages(
                    ddb,
                    metrics,
                    table_name.as_ref(),
                    &uaid,
                    timestamp,
                    10,
                )
                .await?
            } else {
                Default::default()
            }
        };
        // If we didn't get a timestamp off the last query, use the original
        // value if passed one
        let timestamp = response.timestamp.or(timestamp);
        Ok(CheckStorageResponse {
            include_topic: false,
            messages: response.messages,
            timestamp,
        })
    }

    async fn get_user(&self, uaid: &Uuid) -> ApiResult<Option<UserRecord>> {
        let ddb = self.db_client.clone();
        let result = commands::get_uaid(ddb, uaid, &self.router_table_name).await?;
        if let Some(item) = result.item {
            let user: UserRecord = serde_dynamodb::from_hashmap(item).map_err(|e| {
                ApiErrorKind::DatabaseError(format!("Error deserializing {}", e.to_string()))
            })?;
            return Ok(Some(user));
        }
        return Ok(None);
    }

    /// Remove the node ID from a user in the router table.
    /// The node ID will only be cleared if `connected_at` matches up
    /// with the item's `connected_at`.
    async fn remove_node_id(
        &self,
        uaid: &Uuid,
        node_id: String,
        connected_at: u64,
    ) -> ApiResult<()> {
        let ddb = self.db_client.clone();
        let update_item = UpdateItemInput {
            key: ddb_item! { uaid: s => uaid.to_simple().to_string() },
            update_expression: Some("REMOVE node_id".to_string()),
            condition_expression: Some("(node_id = :node) and (connected_at = :conn)".to_string()),
            expression_attribute_values: Some(hashmap! {
                ":node".to_string() => val!(S => node_id),
                ":conn".to_string() => val!(N => connected_at.to_string())
            }),
            table_name: self.router_table_name.clone(),
            ..Default::default()
        };

        ddb.update_item(update_item.clone())
            .await
            .map_err(|e| ApiErrorKind::DatabaseError(e.to_string()))?;
        Ok(())
    }
    /// Get the set of channel IDs for a user
    async fn get_user_channels(
        &self,
        uaid: &Uuid,
        message_table: &str,
    ) -> ApiResult<HashSet<Uuid>> {
        Ok(
            commands::all_channels(self.db_client.clone(), uaid, message_table)
                .await
                .map(|channels| {
                    channels
                        .into_iter()
                        .map(|channel| channel.parse().map_err(ApiError::from))
                        .collect::<ApiResult<_>>()
                })
                .map_err(|e| ApiErrorKind::DatabaseError(e.to_string()))??,
        )
    }
}

impl DynamoStorage {
    pub async fn from_opts(
        message_table_name: &str,
        router_table_name: &str,
        metrics: Arc<StatsdClient>,
    ) -> ApiResult<Self> {
        let ddb = if let Ok(endpoint) = env::var("AWS_LOCAL_DYNAMODB") {
            trace!("Using local DDB: {:?}", endpoint);
            DynamoDbClient::new_with(
                HttpClient::new().map_err(|_| "TLS initialization error")?,
                StaticProvider::new_minimal("BogusKey".to_string(), "BogusKey".to_string()),
                Region::Custom {
                    name: "us-east-1".to_string(),
                    endpoint,
                },
            )
        } else {
            trace!("Using remote DDB: {:?}", Region::default());
            DynamoDbClient::new(Region::default())
        };

        let mut message_table_names = list_message_tables(&ddb, message_table_name)
            .await
            .map_err(|_| "Failed to locate message tables")?;
        // Valid message months are the current and last 2 months
        message_table_names.sort_unstable_by(|a, b| b.cmp(a));
        message_table_names.truncate(3);
        message_table_names.reverse();
        let current_message_month = message_table_names
            .last()
            .ok_or("No last message month found")?
            .to_string();
        trace!("Current message month {:?}", current_message_month);
        Ok(Self {
            db_client: ddb,
            metrics,
            router_table_name: router_table_name.to_owned(),
            message_table_names,
            current_message_month,
        })
    }

    pub async fn increment_storage(
        &self,
        table_name: &str,
        uaid: &Uuid,
        timestamp: &str,
    ) -> ApiResult<UpdateItemOutput> {
        let ddb = self.db_client.clone();
        let expiry = sec_since_epoch() + 2 * MAX_EXPIRY;
        let attr_values = hashmap! {
            ":timestamp".to_string() => val!(N => timestamp),
            ":expiry".to_string() => val!(N => expiry),
        };
        let update_input = UpdateItemInput {
            key: ddb_item! {
                uaid: s => uaid.to_simple().to_string(),
                chidmessageid: s => " ".to_string()
            },
            update_expression: Some("SET current_timestamp=:timestamp, expiry=:expiry".to_string()),
            expression_attribute_values: Some(attr_values),
            table_name: table_name.to_string(),
            ..Default::default()
        };

        Ok(ddb
            .update_item(update_input.clone())
            .await
            .map_err(|e| ApiErrorKind::DatabaseError(e.to_string()))?)
    }
}

pub async fn list_message_tables(ddb: &DynamoDbClient, prefix: &str) -> ApiResult<Vec<String>> {
    let mut names: Vec<String> = Vec::new();
    let mut start_key = None;
    loop {
        let result = commands::list_tables(ddb, start_key).await?;
        start_key = result.last_evaluated_table_name;
        if let Some(table_names) = result.table_names {
            names.extend(table_names);
        }
        if start_key.is_none() {
            break;
        }
    }
    let names = names
        .into_iter()
        .filter(|name| name.starts_with(prefix))
        .collect();
    trace!("Available tables: {:?}", &names);
    Ok(names)
}
