use std::collections::HashMap;
use std::io::{self, BufRead, Read};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const VERSION: u32 = 1;
pub const MAX_LINE: usize = 256 * 1024;

/// Reads one message without allocating beyond the wire limit plus one byte.
pub fn read_line(reader: &mut impl BufRead, line: &mut Vec<u8>) -> io::Result<usize> {
    line.clear();
    let size = reader.take((MAX_LINE + 1) as u64).read_until(b'\n', line)?;
    if size > MAX_LINE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message exceeds the 256 KiB line limit",
        ));
    }
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unterminated_oversized_messages_before_reading_the_rest() {
        let mut reader = io::Cursor::new(vec![b'x'; MAX_LINE * 2]);
        let mut line = Vec::new();
        assert_eq!(
            read_line(&mut reader, &mut line).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(reader.position(), (MAX_LINE + 1) as u64);
        assert_eq!(line.len(), MAX_LINE + 1);
    }

    #[test]
    fn reads_separate_messages_and_accepts_exact_limit() {
        let mut bytes = vec![b'x'; MAX_LINE - 1];
        bytes.extend_from_slice(b"\n{}\n");
        let mut reader = io::Cursor::new(bytes);
        let mut line = Vec::new();
        assert_eq!(read_line(&mut reader, &mut line).unwrap(), MAX_LINE);
        assert_eq!(read_line(&mut reader, &mut line).unwrap(), 3);
        assert_eq!(line, b"{}\n");
        assert_eq!(read_line(&mut reader, &mut line).unwrap(), 0);
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct MethodSpec {
    /// Descriptive metadata; the host does not enforce read-only execution.
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
        /// Advertised store keys; not a host-enforced schema or subscription filter.
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
