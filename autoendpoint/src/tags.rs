use std::collections::{BTreeMap, HashMap};

use actix_web::{
    dev::{Payload, RequestHead},
    Error, FromRequest, HttpRequest,
};
use futures::future;
use futures::future::Ready;
use serde::{
    ser::{SerializeMap, Serializer},
    Serialize,
};
use serde_json::value::Value;

#[derive(Clone, Debug)]
pub struct Tags {
    pub tags: HashMap<String, String>,
    pub extra: HashMap<String, String>,
}

impl Default for Tags {
    fn default() -> Tags {
        Tags {
            tags: HashMap::new(),
            extra: HashMap::new(),
        }
    }
}

impl Serialize for Tags {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_map(Some(self.tags.len()))?;
        for tag in self.tags.clone() {
            if !tag.1.is_empty() {
                seq.serialize_entry(&tag.0, &tag.1)?;
            }
        }
        seq.end()
    }
}

#[allow(dead_code)]
fn insert_if_not_empty(label: &str, val: &str, tags: &mut HashMap<String, String>) {
    if !val.is_empty() {
        tags.insert(label.to_owned(), val.to_owned());
    }
}

impl Tags {
    pub fn from_request_head(req_head: &RequestHead) -> Tags {
        // Return an Option<> type because the later consumers (ApiErrors) presume that
        // tags are optional and wrapped by an Option<> type.
        let mut tags = HashMap::new();
        tags.insert("uri.method".to_owned(), req_head.method.to_string());
        Tags {
            tags,
            extra: HashMap::new(),
        }
    }

    pub fn with_tags(tags: HashMap<String, String>) -> Tags {
        if tags.is_empty() {
            return Tags::default();
        }
        Tags {
            tags,
            extra: HashMap::new(),
        }
    }

    pub fn get(&self, label: &str) -> String {
        let none = "None".to_owned();
        self.tags.get(label).map(String::from).unwrap_or(none)
    }

    pub fn extend(&mut self, tags: HashMap<String, String>) {
        self.tags.extend(tags);
    }

    pub fn tag_tree(self) -> BTreeMap<String, String> {
        let mut result = BTreeMap::new();

        for (k, v) in self.tags {
            result.insert(k.clone(), v.clone());
        }
        result
    }

    pub fn extra_tree(self) -> BTreeMap<String, Value> {
        let mut result = BTreeMap::new();

        for (k, v) in self.extra {
            result.insert(k.clone(), Value::from(v));
        }
        result
    }
}

impl slog::KV for Tags {
    fn serialize(
        &self,
        _record: &slog::Record<'_>,
        serializer: &mut dyn slog::Serializer,
    ) -> slog::Result {
        for (key, val) in &self.tags {
            serializer.emit_str(slog::Key::from(key.clone()), &val)?;
        }
        Ok(())
    }
}

impl FromRequest for Tags {
    type Config = ();
    type Error = Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let tags = {
            let exts = req.extensions();
            match exts.get::<Tags>() {
                Some(t) => t.clone(),
                None => Tags::from_request_head(req.head()),
            }
        };

        future::ok(tags)
    }
}

impl Into<BTreeMap<String, String>> for Tags {
    fn into(self) -> BTreeMap<String, String> {
        let mut result = BTreeMap::new();

        for (k, v) in self.tags {
            result.insert(k.clone(), v.clone());
        }

        result
    }
}
