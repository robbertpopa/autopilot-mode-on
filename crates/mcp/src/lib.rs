//! A Model Context Protocol server over standard input and output, so another
//! agent can read tickets and files through Autopilot without being handed the
//! whole machine.
//!
//! The server refuses to start without at least one read root and one write
//! root, and refuses every file path that resolves outside them. Links are
//! followed only to see where they lead, and a path that leads out is refused.
//! Tools: `list_tickets`, `ticket_status`, `ticket_history`, `waiting_requests`,
//! `answer_request`, `read_file`, `write_file`. `answer_request` goes through the
//! same inbox and checks as the window, and never approves a forced
//! confirmation: only the owner can.
//! `approve` is the tool Claude Code calls through `--permission-prompt-tool`:
//! it puts the agent's request in the inbox and waits for the owner. It can only
//! ask. The answer is allow with the input unchanged, or deny; a wait that ends
//! without a plain approval, and anything it cannot show whole, is a deny.
//! Nothing here starts an agent or spends money.

mod scope;

use std::{
    io::{BufRead, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

use autopilot_core::{RequestId, TicketId};
use autopilot_pipeline::{
    clarify::Choice,
    inbox::{Decision, Inbox, Reply, Request, RequestBody, check_reply},
};
use serde_json::{Value, json};

pub use scope::Scope;

const PROTOCOL: &str = "2024-11-05";
/// The most a read returns, so one call cannot fill another agent's context.
pub const MAX_READ_BYTES: u64 = 256 * 1024;
/// The most a write accepts.
pub const MAX_WRITE_BYTES: usize = 1024 * 1024;
/// A history call returns the latest lines only.
const HISTORY_LINES: usize = 100;
/// A line from a ticket or an agent is data of unknown size.
const LINE_CHARS: usize = 500;
/// How often a waiting `approve` looks for the owner's answer.
const ANSWER_POLL: Duration = Duration::from_millis(50);
/// Prefix of the inbox id of a permission request, so it is told apart from the
/// engine's own requests.
pub const PERMISSION_PREFIX: &str = "perm-";

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
    prompt: Option<PermissionPrompt>,
}

/// Where a permission request goes and how long it may wait.
struct PermissionPrompt {
    ticket: TicketId,
    wait: Duration,
}

impl Server {
    /// # Errors
    /// Returns an error if either root list is empty or a root cannot be used.
    pub fn new(data: PathBuf, reads: &[PathBuf], writes: &[PathBuf]) -> Result<Self, Error> {
        Ok(Self {
            data,
            scope: Scope::new(reads, writes)?,
            prompt: None,
        })
    }

    /// Let `approve` ask the owner about this ticket. Without it, `approve`
    /// denies everything, so a server started for files alone never grants a
    /// tool call.
    #[must_use]
    pub fn with_permission_prompt(mut self, ticket: TicketId, wait: Duration) -> Self {
        self.prompt = Some(PermissionPrompt { ticket, wait });
        self
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

    /// `answer` is `approve` or `reject` for an approval, and `recommended`
    /// for a question.
    fn answer(&self, request: &str, answer: &str) -> Result<String, Error> {
        let refuse = |why: &str| Err(Error::Refused(why.to_owned()));
        let inbox = Inbox::in_data_dir(&self.data);
        let id = RequestId::from(request);
        let waiting = inbox
            .read_request(&id)
            .map_err(|error| Error::Refused(error.to_string()))?
            .ok_or_else(|| Error::Refused("no such waiting request".to_owned()))?;
        let reply = match (&waiting.body, answer) {
            (RequestBody::Question { questions }, "recommended") => Reply::Question {
                choices: vec![Choice::Recommended; questions.len()],
            },
            (RequestBody::Approval { forced: true, .. }, "approve") => {
                // A client is not the owner: a forced confirmation exists so a
                // person looks at the exact action first.
                return refuse("this approval is forced; only the owner can approve it");
            }
            (RequestBody::Approval { .. }, "approve" | "reject") => Reply::Approval {
                decision: if answer == "approve" {
                    Decision::Approve
                } else {
                    Decision::Reject
                },
                allow_for_project: false,
            },
            (RequestBody::Question { .. }, _) => {
                return refuse("answer a question with \"recommended\"");
            }
            (RequestBody::Approval { .. }, _) => {
                return refuse("answer an approval with \"approve\" or \"reject\"");
            }
        };
        check_reply(&waiting, &reply).map_err(|error| Error::Refused(error.to_string()))?;
        inbox
            .write_answer(&id, &reply)
            .map_err(|error| Error::Refused(error.to_string()))?;
        // The engine's event log knows nothing of a permission request, and a
        // sync would delete its files before `approve` has read the answer.
        if !request.starts_with(PERMISSION_PREFIX) {
            autopilot_engine::sync_inbox(&self.data, &waiting.ticket)
                .map_err(|error| Error::Refused(error.to_string()))?;
        }
        Ok(format!("answered {request}"))
    }

    /// The reply Claude Code expects from a permission prompt tool: JSON text
    /// with `behavior` `allow` (and the input to run) or `deny` (and why).
    fn approve(&self, arguments: &Value) -> Value {
        match self.ask_owner(arguments) {
            Ok(input) => json!({"behavior": "allow", "updatedInput": input}),
            Err(why) => json!({"behavior": "deny", "message": why}),
        }
    }

    // Every way out except a plain Approve is a deny, so a bug here fails
    // closed. The input goes back as it came: a prompt tool that rewrote it
    // could widen what the agent runs beyond what the owner saw.
    fn ask_owner(&self, arguments: &Value) -> Result<Value, String> {
        let prompt = self
            .prompt
            .as_ref()
            .ok_or("this server is not set up to approve tool calls")?;
        let tool = arguments
            .get("tool_name")
            .and_then(Value::as_str)
            .ok_or("the request names no tool")?;
        let input = arguments.get("input").ok_or("the request has no input")?;
        let call = arguments
            .get("tool_use_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        let action = format!("let the agent run {tool} with {input}");
        // The owner approves what they can read; a cut-off action is not that.
        if action.chars().count() > LINE_CHARS {
            return Err("the call is too long to show to the owner".to_owned());
        }
        let id = RequestId::from(format!("{PERMISSION_PREFIX}{call}"));
        let inbox = Inbox::in_data_dir(&self.data);
        let request = Request {
            id: id.clone(),
            ticket: prompt.ticket.clone(),
            body: RequestBody::Approval {
                action,
                forced: true,
            },
        };
        inbox
            .write_request(&request)
            .map_err(|error| format!("could not ask the owner: {error}"))?;
        let answer = wait_for_answer(&inbox, &id, prompt.wait);
        // A stale answer must not satisfy a later call that reuses the id.
        let _ = inbox.clean_up(&id);
        match answer {
            Some(Reply::Approval {
                decision: Decision::Approve,
                ..
            }) => Ok(input.clone()),
            Some(_) => Err("the owner did not approve this call".to_owned()),
            None => Err("the owner did not answer in time".to_owned()),
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
            "ticket_history" => {
                let id = TicketId::from(text("ticket")?.to_owned());
                let view = autopilot_engine::load_ticket(&self.data, &id)
                    .map_err(|error| Error::Refused(error.to_string()))?;
                let skip = view.timeline.len().saturating_sub(HISTORY_LINES);
                Ok(view
                    .timeline
                    .iter()
                    .skip(skip)
                    .map(|line| format!("{} {}", line.at_ms, bounded(&line.text)))
                    .collect::<Vec<_>>()
                    .join(
                        "
",
                    ))
            }
            "waiting_requests" => {
                let requests = Inbox::in_data_dir(&self.data)
                    .requests()
                    .map_err(|error| Error::Refused(error.to_string()))?;
                Ok(requests
                    .iter()
                    .map(|request| match &request.body {
                        RequestBody::Question { questions } => format!(
                            "{} {} question ({} to answer)",
                            request.id.as_str(),
                            request.ticket.as_str(),
                            questions.len()
                        ),
                        RequestBody::Approval { action, forced } => format!(
                            "{} {} approval{}: {}",
                            request.id.as_str(),
                            request.ticket.as_str(),
                            if *forced { " (forced, owner only)" } else { "" },
                            bounded(action)
                        ),
                    })
                    .collect::<Vec<_>>()
                    .join(
                        "
",
                    ))
            }
            "answer_request" => self.answer(text("request")?, text("answer")?),
            "approve" => Ok(self.approve(arguments).to_string()),
            "read_file" => self.scope.read(text("path")?),
            "write_file" => self.scope.write(text("path")?, text("text")?),
            other => Err(Error::Refused(format!("no tool {other}"))),
        }
    }
}

fn wait_for_answer(inbox: &Inbox, id: &RequestId, wait: Duration) -> Option<Reply> {
    let start = Instant::now();
    loop {
        if let Ok(Some(reply)) = inbox.read_answer(id) {
            return Some(reply);
        }
        if start.elapsed() >= wait {
            return None;
        }
        std::thread::sleep(ANSWER_POLL);
    }
}

/// One line, control characters removed, cut to a fixed length.
fn bounded(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(LINE_CHARS)
        .collect()
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
        {"name": "ticket_history", "description": "Show the latest timeline lines of a ticket.",
         "inputSchema": object(&json!({"ticket": {"type": "string"}}), &["ticket"])},
        {"name": "waiting_requests", "description": "List what is waiting for the owner: questions and approvals.",
         "inputSchema": object(&json!({}), &[])},
        {"name": "answer_request", "description": "Answer a waiting request: \"recommended\" for a question, \"approve\" or \"reject\" for an approval. A forced approval can only be rejected.",
         "inputSchema": object(&json!({"request": {"type": "string"}, "answer": {"type": "string"}}), &["request", "answer"])},
        {"name": "approve", "description": "Permission prompt for Claude Code (--permission-prompt-tool): asks the owner whether the agent may run one tool call.",
         "inputSchema": object(&json!({"tool_name": {"type": "string"}, "input": {"type": "object"}, "tool_use_id": {"type": "string"}}), &["tool_name", "input"])},
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
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 8);
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

    fn waiting(data: &std::path::Path, id: &str, forced: bool) {
        use autopilot_pipeline::inbox::Request;
        Inbox::in_data_dir(data)
            .write_request(&Request {
                id: RequestId::from(id),
                ticket: TicketId::from("t-1"),
                body: RequestBody::Approval {
                    action: "push".into(),
                    forced,
                },
            })
            .unwrap();
    }

    // Invariant: an MCP client cannot approve a forced request on the owner's
    // behalf. Removing the forced arm in `answer` would let any connected
    // agent publish; the engine accepts a bare Approve file for a forced
    // request, so no other test or check stops it.
    #[test]
    fn a_client_cannot_approve_a_forced_request() {
        let dir = tempfile::tempdir().unwrap();
        waiting(dir.path(), "r1", true);
        let server = server(dir.path(), dir.path());
        let (error, text) = call(
            &server,
            "answer_request",
            &json!({"request": "r1", "answer": "approve"}),
        );
        assert!(error, "{text}");
        assert!(text.contains("forced"), "{text}");
        let inbox = Inbox::in_data_dir(dir.path());
        let id = RequestId::from("r1");
        assert!(inbox.read_answer(&id).unwrap().is_none());
        assert!(inbox.read_request(&id).unwrap().is_some());
    }

    // Invariant: an answer other than the ones named is refused before any
    // file is written, so a typo is never read as consent.
    #[test]
    fn an_unknown_answer_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        waiting(dir.path(), "r2", false);
        let server = server(dir.path(), dir.path());
        for answer in ["yes", "", "recommended"] {
            let (error, _) = call(
                &server,
                "answer_request",
                &json!({"request": "r2", "answer": answer}),
            );
            assert!(error, "{answer}");
        }
        let (error, _) = call(
            &server,
            "answer_request",
            &json!({"request": "../x", "answer": "reject"}),
        );
        assert!(error);
        let id = RequestId::from("r2");
        assert!(
            Inbox::in_data_dir(dir.path())
                .read_answer(&id)
                .unwrap()
                .is_none()
        );
    }

    // Invariant: history returns the ticket's timeline with control characters
    // and over-long text neutralised, because ticket text is data from outside.
    // A change that returned the raw text would let a newline fake a second
    // line to the agent that reads it; the roots tests never touch this tool.
    #[test]
    fn history_is_bounded_and_one_line_per_event() {
        use autopilot_core::{Event, NewEvent};
        use autopilot_store::{SqliteStore, Store};
        let dir = tempfile::tempdir().unwrap();
        let ticket = TicketId::from("t-1");
        let title = format!(
            "a
forged line {}",
            "x".repeat(2000)
        );
        SqliteStore::open(dir.path().join("events.sqlite3"))
            .unwrap()
            .append(
                &ticket,
                0,
                &[NewEvent {
                    at_ms: 5,
                    step: None,
                    kind: Event::TicketCreated { title },
                }],
            )
            .unwrap();
        let server = server(dir.path(), dir.path());
        let (error, text) = call(&server, "ticket_history", &json!({"ticket": "t-1"}));
        assert!(!error, "{text}");
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(text.starts_with("5 Ticket created: a forged line"));
        assert!(text.chars().count() < 600);
        let (error, _) = call(&server, "ticket_history", &json!({"ticket": "nope"}));
        assert!(error);
    }

    // Invariant: a waiting forced approval is shown as owner only, so the
    // client knows it cannot answer it. Dropping the flag from the listing
    // would hide that from the agent; no other test reads the listing.
    #[test]
    fn waiting_requests_name_the_kind_and_mark_forced_approvals() {
        use autopilot_pipeline::inbox::Request;
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::in_data_dir(dir.path());
        inbox
            .write_request(&Request {
                id: RequestId::from("r1"),
                ticket: TicketId::from("t-1"),
                body: RequestBody::Approval {
                    action: "push
the commit"
                        .into(),
                    forced: true,
                },
            })
            .unwrap();
        let server = server(dir.path(), dir.path());
        let (error, text) = call(&server, "waiting_requests", &json!({}));
        assert!(!error);
        assert_eq!(
            text,
            "r1 t-1 approval (forced, owner only): push the commit"
        );
    }

    const PROMPT_FIXTURE: &str =
        include_str!("../../../fixtures/claude/live-2.1.332-permission-prompt-mcp.jsonl");
    const DENY_RUN: &str =
        include_str!("../../../fixtures/claude/live-2.1.332-permission-prompt-deny.jsonl");
    const ALLOW_RUN: &str =
        include_str!("../../../fixtures/claude/live-2.1.332-permission-prompt-allow.jsonl");

    /// The recorded call, and the recorded allow and deny replies.
    fn recorded() -> (String, Value, Value) {
        let lines: Vec<Value> = PROMPT_FIXTURE
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let ask = lines[0]["message"].to_string();
        (
            ask,
            lines[2]["message"].clone(),
            lines[1]["message"].clone(),
        )
    }

    fn prompting(dir: &std::path::Path, wait: Duration) -> Server {
        server(dir, dir).with_permission_prompt(TicketId::from("t-1"), wait)
    }

    /// Plays the owner: waits for the permission request, hands it to `check`,
    /// then writes `reply` as the answer.
    fn owner_answers(
        data: &std::path::Path,
        reply: Reply,
        check: fn(&Request),
    ) -> std::thread::JoinHandle<()> {
        let inbox = Inbox::in_data_dir(data);
        std::thread::spawn(move || {
            for _ in 0..400 {
                if let Some(request) = inbox.requests().unwrap().first() {
                    check(request);
                    inbox.write_answer(&request.id, &reply).unwrap();
                    return;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            panic!("no permission request reached the inbox");
        })
    }

    fn queued(data: &std::path::Path) -> usize {
        Inbox::in_data_dir(data).requests().unwrap().len()
    }

    fn approval(decision: Decision) -> Reply {
        Reply::Approval {
            decision,
            allow_for_project: false,
        }
    }

    fn behavior(reply: &Value) -> Value {
        let text = reply["result"]["content"][0]["text"].as_str().unwrap();
        serde_json::from_str(text).unwrap()
    }

    // Invariant: a tool call from the agent reaches the inbox as a forced
    // approval, and the owner's approval comes back in the shape Claude Code
    // recorded as letting the call run (same id, same input). A change to the
    // reply shape or to what the inbox gets would stall or mis-answer a real
    // run; only the recorded exchange knows the shape.
    #[test]
    fn an_approved_call_reaches_the_inbox_and_the_run_continues() {
        let dir = tempfile::tempdir().unwrap();
        let (asked, allow, _) = recorded();
        let owner = owner_answers(dir.path(), approval(Decision::Approve), |request| {
            assert!(request.id.as_str().starts_with(PERMISSION_PREFIX));
            let RequestBody::Approval { action, forced } = &request.body else {
                panic!("a permission request is an approval");
            };
            assert!(*forced);
            assert!(action.contains("Read"), "{action}");
        });
        let reply = prompting(dir.path(), Duration::from_secs(10))
            .handle(&asked)
            .unwrap();
        owner.join().unwrap();
        assert_eq!(behavior(&reply), behavior(&allow));
        assert_eq!(reply["id"], allow["id"]);
        assert_eq!(queued(dir.path()), 0);
    }

    // Invariant: every answer but a plain approval, and no answer at all, is
    // a deny with no input to run. Widening the match on the decision (or
    // defaulting a timeout to allow) would let a call the owner refused or
    // never saw run.
    #[test]
    fn a_denied_call_stays_denied() {
        let (asked, _, deny) = recorded();
        let check = |reply: &Value| {
            let decision = behavior(reply);
            assert_eq!(decision["behavior"], "deny", "{decision}");
            assert!(decision.get("updatedInput").is_none());
            assert!(decision["message"].is_string());
            assert_eq!(reply["id"], deny["id"]);
        };
        for decision in [
            Decision::Reject,
            Decision::Edit("rm -rf /".into()),
            Decision::SendBack("again".into()),
            Decision::ApproveCorrected(vec![]),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let owner = owner_answers(dir.path(), approval(decision), |_| {});
            let reply = prompting(dir.path(), Duration::from_secs(10))
                .handle(&asked)
                .unwrap();
            owner.join().unwrap();
            check(&reply);
        }
        let dir = tempfile::tempdir().unwrap();
        let reply = prompting(dir.path(), Duration::from_millis(100))
            .handle(&asked)
            .unwrap();
        check(&reply);
    }

    // Invariant: the prompt tool can only ask. Without a ticket it denies and
    // writes nothing; an allow returns exactly the input it was given and
    // nothing else (no rewritten input, no standing permission); a call too
    // long to show whole is denied unasked; a client cannot approve the
    // request for the owner. A change that echoes a different input or adds
    // `updatedPermissions` would widen what Claude Code lets the agent do.
    #[test]
    fn the_prompt_tool_can_only_ask() {
        let (asked, _, _) = recorded();
        let dir = tempfile::tempdir().unwrap();
        let reply = server(dir.path(), dir.path()).handle(&asked).unwrap();
        assert_eq!(behavior(&reply)["behavior"], "deny");
        assert_eq!(queued(dir.path()), 0);

        let long = json!({"tool_name": "Bash", "tool_use_id": "toolu_2",
            "input": {"command": "x".repeat(2 * LINE_CHARS)}});
        let (error, text) = call(
            &prompting(dir.path(), Duration::from_secs(1)),
            "approve",
            &long,
        );
        assert!(!error);
        assert!(text.contains("deny"), "{text}");
        assert_eq!(queued(dir.path()), 0);

        let input = json!({"command": "ls", "extra": [1, {"a": null}]});
        let ask = json!({"tool_name": "Bash", "tool_use_id": "toolu_3", "input": input});
        let owner = owner_answers(dir.path(), approval(Decision::Approve), |_| {});
        let (_, text) = call(
            &prompting(dir.path(), Duration::from_secs(10)),
            "approve",
            &ask,
        );
        owner.join().unwrap();
        let decision: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            decision,
            json!({"behavior": "allow", "updatedInput": input})
        );

        let dir = tempfile::tempdir().unwrap();
        let waiting = Request {
            id: RequestId::from("perm-toolu_4"),
            ticket: TicketId::from("t-1"),
            body: RequestBody::Approval {
                action: "run".into(),
                forced: true,
            },
        };
        let inbox = Inbox::in_data_dir(dir.path());
        inbox.write_request(&waiting).unwrap();
        let (error, _) = call(
            &server(dir.path(), dir.path()),
            "answer_request",
            &json!({"request": "perm-toolu_4", "answer": "approve"}),
        );
        assert!(error);
        assert!(inbox.read_answer(&waiting.id).unwrap().is_none());
    }

    // Invariant: what the recorded runs show about the two answers: a deny
    // gives the model a tool error and a listed denial, an allow gives it the
    // file's text and no denial, and both runs go on to a normal result. If
    // Claude Code changed this, the shapes above would no longer mean what
    // these tests say they mean.
    #[test]
    fn the_recorded_runs_show_what_each_answer_does() {
        for (run, denied) in [(DENY_RUN, true), (ALLOW_RUN, false)] {
            let frames: Vec<Value> = run
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            let result = frames.last().unwrap();
            assert_eq!(result["type"], "result");
            assert_eq!(result["subtype"], "success");
            let denials = result["permission_denials"].as_array().unwrap();
            assert_eq!(denials.len(), usize::from(denied));
            let tool_result = frames
                .iter()
                .flat_map(|f| f["message"]["content"].as_array().into_iter().flatten())
                .find(|block| block["type"] == "tool_result")
                .unwrap();
            assert_eq!(tool_result["is_error"].as_bool().unwrap_or(false), denied);
        }
    }
}
