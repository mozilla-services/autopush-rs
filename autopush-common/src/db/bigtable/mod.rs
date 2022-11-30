/// TODO: Build out the BigTable integration based off of the
/// autopush-bt::bittable_client code.
///
///
mod bigtable_client;

pub use bigtable_client::error::BigTableError;
pub use bigtable_client::BigTableClientImpl;
