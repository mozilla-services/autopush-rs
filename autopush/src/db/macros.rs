/// A bunch of macro helpers from rusoto_helpers code, which they pulled from crates.io because
/// they were waiting for rusuto to hit 1.0.0 or something. For sanity, they are instead accumulated
/// here for our use.
#[allow(unused_macros)]
macro_rules! attributes {
    ($($val:expr => $attr_type:expr),*) => {
        {
            let mut temp_vec = Vec::new();
            $(
                temp_vec.push(AttributeDefinition {
                    attribute_name: String::from($val),
                    attribute_type: String::from($attr_type)
                });
            )*
            temp_vec
        }
    }
}

#[allow(unused_macros)]
macro_rules! key_schema {
    ($($name:expr => $key_type:expr),*) => {
        {
            let mut temp_vec = Vec::new();
            $(
                temp_vec.push(KeySchemaElement {
                    key_type: String::from($key_type),
                    attribute_name: String::from($name)
                });
            )*
            temp_vec
        }
    }
}

#[macro_export]
macro_rules! val {
    (B => $val:expr) => {
        AttributeValue {
            b: Some($val),
            ..Default::default()
        }
    };
    (S => $val:expr) => {
        AttributeValue {
            s: Some($val.to_string()),
            ..Default::default()
        }
    };
    (SS => $val:expr) => {
        AttributeValue {
            ss: Some($val.iter().map(|v| v.to_string()).collect()),
            ..Default::default()
        }
    };
    (N => $val:expr) => {
        AttributeValue {
            n: Some($val.to_string()),
            ..Default::default()
        }
    };
}

/// Create a **HashMap** from a list of key-value pairs
///
/// ## Example
///
/// ```ignore
/// use autopush_common::hashmap;
///
/// let map = hashmap!{
///     "a" => 1,
///     "b" => 2,
/// };
/// assert_eq!(map["a"], 1);
/// assert_eq!(map["b"], 2);
/// assert_eq!(map.get("c"), None);
/// ```
#[macro_export]
macro_rules! hashmap {
    (@single $($x:tt)*) => (());
    (@count $($rest:expr),*) => (<[()]>::len(&[$($crate::hashmap!(@single $rest)),*]));

    ($($key:expr => $value:expr,)+) => { $crate::hashmap!($($key => $value),+) };
    ($($key:expr => $value:expr),*) => {
        {
            let _cap = $crate::hashmap!(@count $($key),*);
            let mut _map = ::std::collections::HashMap::with_capacity(_cap);
            $(
                _map.insert($key, $value);
            )*
            _map
        }
    };
}

/// Shorthand for specifying a dynamodb item
#[macro_export]
macro_rules! ddb_item {
    ($($p:tt: $t:tt => $x:expr),*) => {
        {
            use rusoto_dynamodb::AttributeValue;
            $crate::hashmap!{
                $(
                    String::from(stringify!($p)) => AttributeValue {
                        $t: Some($x),
                        ..Default::default()
                    },
                )*
            }
        }
    }
}
