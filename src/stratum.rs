mod server;

use serde::{de, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub use server::Stratum;
use std::borrow::Cow;
use std::fmt;

enum Id {
    Number(u64),
    Text(Box<str>),
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

enum Response<T> {
    Ok(OkResponse<T>),
    Err(ErrResponse),
}

impl<T> Serialize for Response<T>
where
    T: Serialize,
{
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
struct OkResponse<T> {
    id: Id,
    result: T,
}

#[derive(Serialize)]
struct ErrResponse {
    id: Id,
    error: Error,
}

#[derive(Serialize)]
struct Error {
    code: u64,
    message: String,
}
