use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const VERSION: u32 = 1;
pub const MAX_LINE: usize = 256 * 1024;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct MethodSpec {
    #[serde(default)]
    pub readonly: bool,
}

/// Messages sent by an extension to riced.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ToHost {
    Register {
        v: u32,
        name: String,
        #[serde(default)]
        state: Vec<String>,
        #[serde(default)]
        methods: HashMap<String, MethodSpec>,
        #[serde(default)]
        pid: Option<u32>,
    },
    State {
        store: Value,
    },
    Event {
        event: Value,
    },
    Result {
        id: u64,
        ok: bool,
        #[serde(default)]
        result: Value,
        #[serde(default)]
        error: Option<String>,
    },
}

/// Messages sent by riced to an extension.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ToPlugin {
    Call {
        id: u64,
        method: String,
        #[serde(default)]
        args: Vec<Value>,
    },
    /// Fields the Lua store has read since the last message. Extensions may use this to
    /// skip work for data nothing asks for.
    Subscribe {
        fields: Vec<String>,
    },
    Error {
        error: String,
    },
}
