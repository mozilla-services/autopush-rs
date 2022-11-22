/// TODO: Build out the BigTable integration based off of the
/// autopush-bt::bittable_client code.
///
///

mod bigtable_client;

#[allow(dead_code)] // TODO: Remove before flight
#[derive(Clone)]
pub struct BigTableClientImpl {
    db_client: bigtable_client::BigTableClient,
    _metrics: Arc<StatsdClient>,
    router_table: String,                  // Routing information
    message_table: String,                 // Message storage information
    meta_table: String,                    // Channels and meta info for a user.
    current_message_month: Option<String>, // For table rotation
}


impl BigTableClientImpl {
    
}
