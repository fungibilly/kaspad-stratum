mod jobs;
mod server;

use anyhow::Result;
use serde::{de, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
pub use server::Stratum;
use std::borrow::Cow;
use std::fmt;

#[derive(Clone)]
pub enum Id {
    Number(u64),
    Text(Box<str>),
}

impl From<u64> for Id {
    fn from(v: u64) -> Self {
        Id::Number(v)
    }
}

struct IdVisitor;
impl<'de> de::Visitor<'de> for IdVisitor {
    type Value = Id;
    fn expecting(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.write_str("an integer or string")
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Id::Number(v))
    }

    fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Id::Text(v.into_boxed_str()))
    }
}

impl<'de> Deserialize<'de> for Id {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        d.deserialize_u64(IdVisitor)
    }
}

impl Serialize for Id {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Id::Number(v) => s.serialize_u64(*v),
            Id::Text(v) => s.serialize_str(v),
        }
    }
}

#[derive(Deserialize, Serialize)]
struct Request {
    #[serde(default)]
    id: Option<Id>,
    method: Cow<'static, str>,
    #[serde(default)]
    params: Option<Value>,
}

pub enum Response {
    Ok(OkResponse),
    Err(ErrResponse),
}

impl Response {
    pub fn ok<T: Serialize>(id: Id, result: T) -> Result<Self> {
        Ok(Self::Ok(OkResponse {
            id,
            result: serde_json::to_value(result)?,
        }))
    }

    pub fn err(id: Id, code: u64, message: Box<str>) -> Result<Self> {
        Ok(Self::Err(ErrResponse {
            id,
            error: json!((code, message, ())),
        }))
    }
}

impl Serialize for Response {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Response::Ok(v) => v.serialize(s),
            Response::Err(v) => v.serialize(s),
        }
    }
}

#[derive(Serialize)]
pub struct OkResponse {
    id: Id,
    result: Value,
}

#[derive(Serialize)]
pub struct ErrResponse {
    id: Id,
    error: Value,
}
