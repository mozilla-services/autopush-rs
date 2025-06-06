use bytestring::ByteString;
use uuid::Uuid;

use crate::protocol::MessageType;
use autopush_common::{
    db::{mock::MockDbClient, User},
    util::timing::ms_since_epoch,
};

pub const UA: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:109.0) Gecko/20100101 Firefox/110.0";

pub const DUMMY_UAID: Uuid = Uuid::from_u128(0xdeadbeef_0000_0000_deca_fbad00000000);
pub const DUMMY_CHID: Uuid = Uuid::from_u128(0xdeadbeef_0000_0000_abad_1dea00000000);

/// A minimal websocket Push "hello" message, used by an unregistered UA with
/// no existing channel subscriptions
pub fn hello_json() -> ByteString {
    format!(
        r#"{{"messageType": "{}", "use_webpush": true}}"#,
        MessageType::Hello.as_ref()
    )
    .into()
}

pub fn hello_again_json() -> ByteString {
    format!(
        r#"{{"messageType": "{}", "use_webpush": true,
                "uaid": "{}"}}"#,
        MessageType::Hello.as_ref(),
        DUMMY_UAID
    )
    .into()
}

pub const CURRENT_MONTH: &str = "message_2018_06";

/// Return a simple MockDbClient that responds to hello (once) with a new uaid.
pub fn hello_db() -> MockDbClient {
    MockDbClient::new()
}

/// Return a simple MockDbClient that responds to hello (once) with the
/// specified uaid.
pub fn hello_again_db(uaid: Uuid) -> MockDbClient {
    let mut db = MockDbClient::new();
    db.expect_get_user().times(1).return_once(move |_| {
        let user = User::builder()
            .uaid(uaid)
            .connected_at(ms_since_epoch() - (10 * 60 * 1000))
            .build()
            .unwrap();
        Ok(Some(user))
    });

    db.expect_update_user().times(1).return_once(|_| Ok(true));
    db.expect_fetch_topic_messages()
        .times(1)
        .return_once(|_, _| Ok(Default::default()));
    db.expect_fetch_timestamp_messages()
        .times(1)
        .return_once(|_, _, _| Ok(Default::default()));
    db
}
