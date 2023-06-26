use uuid::Uuid;

use autopush_common::db::{mock::MockDbClient, User};

pub const UA: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:109.0) Gecko/20100101 Firefox/110.0";

pub const DUMMY_UAID: Uuid = Uuid::from_u128(0xdeadbeef_0000_0000_deca_fbad00000000);
pub const DUMMY_CHID: Uuid = Uuid::from_u128(0xdeadbeef_0000_0000_abad_1dea00000000);

/// A minimal websocket Push "hello" message, used by an unregistered UA with
/// no existing channel subscriptions
pub const HELLO: &str = r#"{"messageType": "hello", "use_webpush": true}"#;
/// A post initial registration response
pub const HELLO_AGAIN: &str = r#"{"messageType": "hello", "use_webpush": true,
                                  "uaid": "deadbeef-0000-0000-deca-fbad00000000"}"#;

pub const CURRENT_MONTH: &str = "message_2018_06";

/// Return a simple MockDbClient that responds to hello (once) with a new uaid.
pub fn hello_db() -> MockDbClient {
    let mut db = MockDbClient::new();
    db.expect_rotating_message_table()
        .times(1)
        .return_const(Some(CURRENT_MONTH));
    db
}

/// Return a simple MockDbClient that responds to hello (once) with the
/// specified uaid.
pub fn hello_again_db(uaid: Uuid) -> MockDbClient {
    let mut db = MockDbClient::new();
    db.expect_get_user().times(1).return_once(move |_| {
        Ok(Some(User {
            uaid,
            current_month: Some(CURRENT_MONTH.to_owned()),
            ..Default::default()
        }))
    });
    db.expect_rotating_message_table()
        .times(1)
        .return_const(Some(CURRENT_MONTH));
    db.expect_update_user().times(1).return_once(|_| Ok(()));
    db.expect_fetch_messages()
        .times(1)
        .return_once(|_, _| Ok(Default::default()));
    db.expect_fetch_timestamp_messages()
        .times(1)
        .return_once(|_, _, _| Ok(Default::default()));
    db
}
