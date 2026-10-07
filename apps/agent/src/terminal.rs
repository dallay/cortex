use agent_core::{
    AgentError, ApprovalPolicy, ApprovalRequest, CancellationToken, Event, EventSink, Result,
};
use async_trait::async_trait;
use std::{
    collections::BTreeSet,
    io::{BufRead, Write},
    sync::Arc,
};

pub struct Input {
    receiver: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<std::io::Result<String>>>,
}
impl Input {
    pub fn new() -> Self {
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        // A single reader owns stdin across prompts and approvals. A detached OS thread
        // avoids Tokio's blocking-stdin shutdown hang while waiting for the next line.
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                if sender.blocking_send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            receiver: tokio::sync::Mutex::new(receiver),
        }
    }
    pub async fn line(&self) -> Result<Option<String>> {
        self.receiver
            .lock()
            .await
            .recv()
            .await
            .transpose()
            .map_err(AgentError::Io)
    }
}
pub struct Policy {
    pub input: Option<Arc<Input>>,
    pub allowed: BTreeSet<String>,
}
#[async_trait]
impl ApprovalPolicy for Policy {
    async fn approve(&self, request: &ApprovalRequest, cancel: CancellationToken) -> Result<bool> {
        eprintln!(
            "\nApproval for {}:\n{}",
            safe(&request.action),
            safe(&request.preview)
        );
        if self.allowed.contains(&request.action) {
            eprintln!("Authorized by --allow for this invocation.");
            return Ok(true);
        }
        let Some(input) = &self.input else {
            eprintln!("Denied: no interactive approval or explicit action grant.");
            return Ok(false);
        };
        eprint!("Approve once? [y/N] ");
        std::io::stderr().flush()?;
        let line = tokio::select! {_=cancel.cancelled()=>return Err(AgentError::Cancelled),line=input.line()=>line?};
        Ok(line
            .as_deref()
            .is_some_and(|s| matches!(s.trim(), "y" | "Y" | "yes")))
    }
}
pub struct Output {
    pub json: bool,
}
impl EventSink for Output {
    fn emit(&self, event: Event) {
        if self.json {
            if let Ok(encoded) = serde_json::to_string(&event) {
                println!("{encoded}");
            }
            return;
        }
        match event {
            Event::Text { text } => {
                print!("{}", safe(&text));
                let _ = std::io::stdout().flush();
            }
            Event::ToolStarted { call } => eprintln!("\nTool: {}", safe(&call.name)),
            Event::ToolFinished {
                is_error: true,
                output,
                ..
            } => eprintln!("\nTool failed: {}", safe(&output)),
            Event::Compacted { .. } => eprintln!("\nContext compacted; original history retained."),
            Event::TurnFinished => println!(),
            _ => {}
        }
    }
}
fn safe(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}
