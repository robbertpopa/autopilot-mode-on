//! A Model Context Protocol server over standard input and output, so another
//! agent can read tickets and files through Autopilot without being handed the
//! whole machine.
//!
//! The server refuses to start without at least one read root and one write
//! root, and refuses every file path that resolves outside them. Links are
//! followed only to see where they lead, and a path that leads out is refused.
//! Tools: `list_tickets`, `ticket_status`, `read_file`, `write_file`.
//! Nothing here starts an agent or spends money.

mod scope;

use std::{
    io::{BufRead, Write},
    path::PathBuf,
};

use autopilot_core::TicketId;
use serde_json::{Value, json};

pub use scope::Scope;

const PROTOCOL: &str = "2024-11-05";
/// The most a read returns, so one call cannot fill another agent's context.
pub const MAX_READ_BYTES: u64 = 256 * 1024;
/// The most a write accepts.
pub const MAX_WRITE_BYTES: usize = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Refused(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub struct Server {
    data: PathBuf,
    scope: Scope,
}

impl Server {
    /// # Errors
    /// Returns an error if either root list is empty or a root cannot be used.
    pub fn new(data: PathBuf, reads: &[PathBuf], writes: &[PathBuf]) -> Result<Self, Error> {
        Ok(Self {
            data,
            scope: Scope::new(reads, writes)?,
        })
    }

    /// Answer requests, one JSON object per line, until the input ends.
    ///
    /// # Errors
    /// Returns an error if reading or writing the streams fails.
    pub fn serve(&self, input: impl BufRead, mut output: impl Write) -> Result<(), Error> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Some(reply) = self.handle(&line) {
                writeln!(output, "{reply}")?;
                output.flush()?;
            }
        }
        Ok(())
    }

    /// One request in, the reply out. A notification has no reply.
    #[must_use]
    pub fn handle(&self, line: &str) -> Option<Value> {
        let Ok(request) = serde_json::from_str::<Value>(line) else {
            return Some(failure(&Value::Null, -32700, "not valid JSON"));
        };
        let id = request.get("id").cloned()?;
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let reply = match method {
            "initialize" => json!({
                "protocolVersion": PROTOCOL,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "autopilot", "version": env!("CARGO_PKG_VERSION")},
            }),
            "ping" => json!({}),
            "tools/list" => json!({"tools": tools()}),
            "tools/call" => {
                let params = request.get("params").cloned().unwrap_or(Value::Null);
                self.call(&params)
            }
            other => return Some(failure(&id, -32601, &format!("no method {other}"))),
        };
        Some(json!({"jsonrpc": "2.0", "id": id, "result": reply}))
    }

    /// A tool's own failure is a result the agent can read, not a protocol error.
    fn call(&self, params: &Value) -> Value {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
        match self.run(name, &arguments) {
            Ok(text) => json!({"content": [{"type": "text", "text": text}]}),
            Err(error) => {
                json!({"isError": true, "content": [{"type": "text", "text": error.to_string()}]})
            }
        }
    }

    fn run(&self, name: &str, arguments: &Value) -> Result<String, Error> {
        let text = |key: &str| {
            arguments
                .get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Refused(format!("the argument {key} is missing")))
        };
        match name {
            "list_tickets" => {
                let ids = autopilot_engine::tickets(&self.data)
                    .map_err(|error| Error::Refused(error.to_string()))?;
                Ok(ids
                    .iter()
                    .map(TicketId::as_str)
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "ticket_status" => {
                let id = TicketId::from(text("ticket")?.to_owned());
                let view = autopilot_engine::load_ticket(&self.data, &id)
                    .map_err(|error| Error::Refused(error.to_string()))?;
                let mut lines = vec![format!("{} ({:?})", view.title, view.badge)];
                for step in &view.steps {
                    lines.push(format!(
                        "{}: {:?}",
                        autopilot_engine::step_name(step.step),
                        step.state
                    ));
                }
                Ok(lines.join("\n"))
            }
            "read_file" => self.scope.read(text("path")?),
            "write_file" => self.scope.write(text("path")?, text("text")?),
            other => Err(Error::Refused(format!("no tool {other}"))),
        }
    }
}

fn failure(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tools() -> Value {
    let object = |properties: &Value, required: &[&str]| json!({"type": "object", "properties": properties, "required": required});
    json!([
        {"name": "list_tickets", "description": "List the ticket ids.",
         "inputSchema": object(&json!({}), &[])},
        {"name": "ticket_status", "description": "Show a ticket's title and where each step stands.",
         "inputSchema": object(&json!({"ticket": {"type": "string"}}), &["ticket"])},
        {"name": "read_file", "description": "Read a text file inside a read root.",
         "inputSchema": object(&json!({"path": {"type": "string"}}), &["path"])},
        {"name": "write_file", "description": "Write a text file inside a write root. The folder must exist.",
         "inputSchema": object(&json!({"path": {"type": "string"}, "text": {"type": "string"}}), &["path", "text"])},
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(read: &std::path::Path, write: &std::path::Path) -> Server {
        Server::new(
            read.to_path_buf(),
            &[read.to_path_buf()],
            &[write.to_path_buf()],
        )
        .unwrap()
    }

    fn call(server: &Server, name: &str, arguments: &Value) -> (bool, String) {
        let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":name,"arguments":arguments}});
        let reply = server.handle(&request.to_string()).unwrap();
        let result = &reply["result"];
        (
            result["isError"].as_bool().unwrap_or(false),
            result["content"][0]["text"]
                .as_str()
                .unwrap_or("")
                .to_owned(),
        )
    }

    // Invariant: the server will not start without both a read root and a
    // write root, so there is no "allow everything" default to fall into.
    #[test]
    fn it_refuses_to_start_without_roots() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        assert!(Server::new(root.clone(), &[], std::slice::from_ref(&root)).is_err());
        assert!(Server::new(root.clone(), std::slice::from_ref(&root), &[]).is_err());
        assert!(
            Server::new(
                root.clone(),
                std::slice::from_ref(&root),
                std::slice::from_ref(&root)
            )
            .is_ok()
        );
    }

    // Invariant: files inside the roots work, and a path that climbs out with
    // `..` or names another folder is refused for reading and for writing.
    #[test]
    fn files_outside_the_roots_are_refused() {
        let read = tempfile::tempdir().unwrap();
        let write = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(read.path().join("a.txt"), "hello").unwrap();
        std::fs::write(outside.path().join("secret.txt"), "no").unwrap();
        let server = server(read.path(), write.path());

        let ok = call(
            &server,
            "read_file",
            &json!({"path": read.path().join("a.txt")}),
        );
        assert_eq!(ok, (false, "hello".to_owned()));

        let climbing = read
            .path()
            .join("..")
            .join(outside.path().file_name().unwrap());
        let up = call(
            &server,
            "read_file",
            &json!({"path": climbing.join("secret.txt")}),
        );
        assert!(up.0, "{up:?}");
        let other = call(
            &server,
            "read_file",
            &json!({"path": outside.path().join("secret.txt")}),
        );
        assert!(other.0);

        let created = call(
            &server,
            "write_file",
            &json!({"path": write.path().join("n.txt"), "text": "x"}),
        );
        assert!(!created.0, "{created:?}");
        assert_eq!(
            std::fs::read_to_string(write.path().join("n.txt")).unwrap(),
            "x"
        );
        let bad = call(
            &server,
            "write_file",
            &json!({"path": outside.path().join("n.txt"), "text": "x"}),
        );
        assert!(bad.0);
        assert!(!outside.path().join("n.txt").exists());
        // A read root is not a write root.
        let into_read = call(
            &server,
            "write_file",
            &json!({"path": read.path().join("n.txt"), "text": "x"}),
        );
        assert!(into_read.0);
    }

    // Invariant: the protocol basics answer, and a notification gets no reply.
    #[test]
    fn it_speaks_the_protocol_basics() {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path(), dir.path());
        let init = server
            .handle(r#"{"jsonrpc":"2.0","id":7,"method":"initialize"}"#)
            .unwrap();
        assert_eq!(init["id"], 7);
        assert_eq!(init["result"]["protocolVersion"], PROTOCOL);
        let list = server
            .handle(r#"{"jsonrpc":"2.0","id":8,"method":"tools/list"}"#)
            .unwrap();
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 4);
        assert!(
            server
                .handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .is_none()
        );
        let unknown = server
            .handle(r#"{"jsonrpc":"2.0","id":9,"method":"nope"}"#)
            .unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
    }
}
