//! Run-owned process generations. Browser contexts never own these resources.
mod config;
mod evidence;
pub(crate) mod hooks;
mod logs;
mod platform;

use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use duhem_actions::{ActionError, ActionResult};
use duhem_evidence::EvidenceWriter;
use serde::Serialize;
use tokio::process::{Child, Command};

use config::{Launch, error, parse};
use logs::{Drain, Logs};

pub(crate) type Shared = Arc<tokio::sync::Mutex<Processes>>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Owner {
    pub run_scope: String,
    pub criterion: Option<String>,
    pub check: Option<String>,
    pub attempt: Option<String>,
    pub fixture: Option<String>,
}

impl Owner {
    pub fn leaf() -> Self {
        Self {
            run_scope: uuid::Uuid::new_v4().to_string(),
            criterion: None,
            check: None,
            attempt: None,
            fixture: None,
        }
    }
    pub fn criterion(&self, id: &str) -> Self {
        Self {
            criterion: Some(id.into()),
            ..self.clone()
        }
    }
    pub fn check(&self, id: &str) -> Self {
        Self {
            check: Some(id.into()),
            attempt: Some(uuid::Uuid::new_v4().to_string()),
            ..self.clone()
        }
    }
    pub fn fixture(&self, name: &str) -> Self {
        Self {
            fixture: Some(name.into()),
            ..self.clone()
        }
    }
    fn may_read(&self, owner: &Self) -> bool {
        self.run_scope == owner.run_scope
            && (owner.criterion.is_none() || self.criterion == owner.criterion)
            && (owner.attempt.is_none() || self.attempt == owner.attempt)
    }
    fn may_mutate(&self, owner: &Self) -> bool {
        self.attempt.is_some() && self.may_read(owner) && self.attempt == owner.attempt
    }
}

#[derive(Debug, Default)]
pub(crate) struct Processes {
    entries: Vec<Entry>,
}

#[derive(Debug)]
struct Entry {
    owner: Owner,
    handle: String,
    generation: u64,
    pid: u32,
    launch: Launch,
    child: Child,
    logs: Arc<Mutex<Logs>>,
    drains: Vec<tokio::task::JoinHandle<()>>,
    exit: Option<platform::Exit>,
    ready: bool,
    stopped: bool,
    stop_result: Option<ActionResult>,
    restarted: bool,
    stop_escalated: bool,
    was_exited: bool,
    finalized: bool,
}

impl Processes {
    pub(super) fn has_entries(&self) -> bool {
        !self.entries.is_empty()
    }

    pub async fn invoke(
        &mut self,
        uses: &'static str,
        with: &serde_yml::Value,
        owner: &Owner,
        child_env: &BTreeMap<String, String>,
        writer: &mut EvidenceWriter,
    ) -> Result<ActionResult, ActionError> {
        if uses == "process/start" {
            let mut launch: Launch = parse(uses, with)?;
            launch.validate()?;
            launch.env.extend(child_env.clone());
            return self.start(launch, owner.clone(), 1, writer).await;
        }
        let (handle, cursor) = if uses == "process/inspect" {
            let with: config::Inspect = parse(uses, with)?;
            (with.handle, with.cursor)
        } else {
            (parse::<config::Handle>(uses, with)?.handle, 0)
        };
        let index = self
            .entries
            .iter()
            .position(|entry| entry.handle == handle)
            .ok_or_else(|| {
                error("unknown process handle (handles belong to one run and generation)")
            })?;
        let entry = &mut self.entries[index];
        if !owner.may_read(&entry.owner) {
            return Err(error(
                "process handle belongs to a sibling check or another retry",
            ));
        }
        if uses != "process/inspect" && !owner.may_mutate(&entry.owner) {
            return Err(error(
                "process/stop and process/restart require a current-check or fixture handle; enclosing handles are read-only",
            ));
        }
        if uses == "process/restart" && entry.restarted {
            return Err(error(
                "process generation has already been restarted; use its replacement handle",
            ));
        }
        let result = match uses {
            "process/inspect" => entry.inspect(cursor),
            "process/stop" | "process/restart" => entry.stop().await,
            _ => return Err(error("unknown process action")),
        };
        entry.record(writer, uses, result.as_ref().err()).await?;
        if uses == "process/restart" {
            result?; // A failed stop must never spawn a replacement.
            let launch = entry.launch.clone();
            let owner = entry.owner.clone();
            let generation = entry.generation + 1;
            entry.restarted = true;
            self.start(launch, owner, generation, writer).await
        } else {
            result
        }
    }

    async fn start(
        &mut self,
        launch: Launch,
        owner: Owner,
        generation: u64,
        writer: &mut EvidenceWriter,
    ) -> Result<ActionResult, ActionError> {
        // Reject before Command::spawn on every unsupported target.
        if !cfg!(any(target_os = "linux", target_os = "macos")) {
            return Err(error(
                "process/start is supported only on Linux and macOS; no process was spawned",
            ));
        }
        let mut command = Command::new(&launch.command);
        command
            .args(&launch.args)
            .env_clear()
            .envs(super::env::sanitized_env_vars(std::env::vars()))
            .envs(&launch.env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(cwd) = &launch.cwd {
            command.current_dir(cwd);
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        command.process_group(0);
        let mut child = command
            .spawn()
            .map_err(|err| error(format!("process/start spawn failed: {err}")))?;
        let pid = child.id().expect("newly spawned Child has a PID");
        let logs = Arc::new(Mutex::new(Logs::default()));
        let started = Instant::now();
        let mut drains = Vec::new();
        let drain_config = |stream| Drain {
            logs: logs.clone(),
            secrets: writer.stream_secrets(),
            stream,
            generation,
            started,
            probe: launch
                .ready
                .as_ref()
                .and_then(|ready| ready.log.as_ref())
                .filter(|log| log.stream == stream)
                .map(|log| regex::Regex::new(&log.pattern).expect("validated regex")),
        };
        drains.push(tokio::spawn(logs::drain(
            child.stdout.take().expect("piped stdout"),
            drain_config("stdout"),
        )));
        drains.push(tokio::spawn(logs::drain(
            child.stderr.take().expect("piped stderr"),
            drain_config("stderr"),
        )));
        self.entries.push(Entry {
            owner,
            handle: uuid::Uuid::new_v4().to_string(),
            generation,
            pid,
            launch,
            child,
            logs,
            drains,
            exit: None,
            ready: false,
            stopped: false,
            stop_result: None,
            restarted: false,
            stop_escalated: false,
            was_exited: false,
            finalized: false,
        });
        // Register before the first await: cancellation/readiness failure still
        // leaves an owned generation for the always-required finalizer.
        let entry = self.entries.last_mut().unwrap();
        let result = entry.readiness().await.map(|()| entry.started_result());
        entry
            .record(writer, "process/start", result.as_ref().err())
            .await?;
        result
    }
}

impl Entry {
    fn observe(&mut self) -> Result<(), ActionError> {
        if !self.stopped && self.exit.is_none() {
            self.exit = platform::observe(self.pid)
                .map_err(|err| error(format!("observe process: {err}")))?;
        }
        Ok(())
    }

    fn started_result(&self) -> ActionResult {
        ActionResult::ok()
            .with_output("handle", self.handle.clone().into())
            .with_output("generation", self.generation.into())
            .with_output("pid", self.pid.into())
            .with_output("ready", self.ready.into())
    }

    fn inspect(&mut self, cursor: u64) -> Result<ActionResult, ActionError> {
        self.observe()?;
        if cursor > self.logs.lock().unwrap().cursor() {
            return Err(error("cursor is beyond this generation's log cursor"));
        }
        let mut result = ActionResult::ok();
        result.outputs = self
            .snapshot(cursor)
            .as_object()
            .unwrap()
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        result.outputs.remove("ready");
        result.outputs.remove("stop");
        Ok(result)
    }

    fn snapshot(&self, cursor: u64) -> serde_json::Value {
        let mut facts = self.logs.lock().unwrap().snapshot(cursor);
        let map = facts.as_object_mut().unwrap();
        map.insert(
            "state".into(),
            (if self.stopped {
                "stopped"
            } else if self.exit.is_some() {
                "exited"
            } else {
                "running"
            })
            .into(),
        );
        map.insert(
            "running".into(),
            (self.exit.is_none() && !self.stopped).into(),
        );
        map.insert(
            "exit_code".into(),
            serde_json::json!(self.exit.and_then(|exit| exit.code)),
        );
        map.insert(
            "signal".into(),
            serde_json::json!(self.exit.and_then(|exit| exit.signal)),
        );
        map.insert("generation".into(), self.generation.into());
        map.insert("ready".into(), self.ready.into());
        if let Some(result) = &self.stop_result {
            map.insert("stop".into(), serde_json::json!(result.outputs));
        }
        facts
    }

    async fn readiness(&mut self) -> Result<(), ActionError> {
        let Some(ready) = self.launch.ready.clone() else {
            return Ok(());
        };
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|err| error(format!("readiness client: {err}")))?;
        let deadline = tokio::time::Instant::now() + ready.timeout.0;
        loop {
            self.observe()?;
            if self.exit.is_some() {
                return Err(error("process exited before required readiness"));
            }
            let matched = if let Some(http) = &ready.http {
                let mut request = client.get(&http.url);
                for (name, value) in &http.headers {
                    request = request.header(name, value);
                }
                match tokio::time::timeout_at(deadline, request.send()).await {
                    Ok(Ok(response)) => response.status().as_u16() == http.status,
                    _ => false,
                }
            } else {
                self.logs.lock().unwrap().ready
            };
            if matched {
                self.observe()?;
                if self.exit.is_some() {
                    return Err(error("process exited before readiness completed"));
                }
                self.ready = true;
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(error("process readiness timed out"));
            }
            tokio::time::sleep_until(
                deadline.min(tokio::time::Instant::now() + Duration::from_millis(25)),
            )
            .await;
        }
    }

    async fn stop(&mut self) -> Result<ActionResult, ActionError> {
        if let Some(result) = &self.stop_result {
            return Ok(result.clone().with_output("already_exited", true.into()));
        }
        if !self.stopped {
            self.observe()?;
            self.was_exited = self.exit.is_some();
            let live = platform::group_live(self.pid)
                .map_err(|err| error(format!("observe owned group: {err}")))?;
            if live {
                platform::signal(self.pid, platform::TERM)
                    .map_err(|err| error(format!("SIGTERM owned group: {err}")))?;
            }
            let graceful = self
                .wait_group(
                    self.launch
                        .stop_timeout
                        .map(|t| t.0)
                        .unwrap_or(Duration::from_secs(5)),
                )
                .await?;
            self.stop_escalated = !graceful;
            if self.stop_escalated {
                platform::signal(self.pid, platform::KILL)
                    .map_err(|err| error(format!("SIGKILL owned group: {err}")))?;
                if !self
                    .wait_group(
                        self.launch
                            .kill_timeout
                            .map(|t| t.0)
                            .unwrap_or(Duration::from_secs(2)),
                    )
                    .await?
                {
                    return Err(error(
                        "owned process group is still live after bounded SIGKILL",
                    ));
                }
            }
            self.observe()?;
            let budget = self
                .launch
                .kill_timeout
                .map(|t| t.0)
                .unwrap_or(Duration::from_secs(2));
            tokio::time::timeout(budget, self.child.wait())
                .await
                .map_err(|_| error("timed out reaping owned child"))?
                .map_err(|err| error(format!("reap owned child: {err}")))?;
            self.stopped = true; // Never signal the PGID after reaping its pinned leader.
        }
        let budget = self
            .launch
            .kill_timeout
            .map(|t| t.0)
            .unwrap_or(Duration::from_secs(2));
        let drains = async {
            while let Some(task) = self.drains.last_mut() {
                task.await
                    .map_err(|err| error(format!("log drain task: {err}")))?;
                self.drains.pop();
            }
            Ok::<_, ActionError>(())
        };
        tokio::time::timeout(budget, drains)
            .await
            .map_err(|_| error("owned process log pipes did not close"))??;
        if !self.logs.lock().unwrap().errors.is_empty() {
            return Err(error(
                "owned process log stream could not be completely drained",
            ));
        }
        let result = ActionResult::ok()
            .with_output("stopped", true.into())
            .with_output("already_exited", self.was_exited.into())
            .with_output("escalated", self.stop_escalated.into())
            .with_output(
                "exit_code",
                serde_json::json!(self.exit.and_then(|exit| exit.code)),
            )
            .with_output(
                "signal",
                serde_json::json!(self.exit.and_then(|exit| exit.signal)),
            );
        self.stop_result = Some(result.clone());
        Ok(result)
    }

    async fn wait_group(&self, budget: Duration) -> Result<bool, ActionError> {
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            if !platform::group_live(self.pid)
                .map_err(|err| error(format!("observe owned group: {err}")))?
            {
                return Ok(true);
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(false);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

impl Drop for Entry {
    fn drop(&mut self) {
        // Last resort for an externally dropped Engine future: no fabricated
        // cleanup success. Normal completion/cancellation uses recorded stop.
        if !self.stopped {
            let _ = platform::signal(self.pid, platform::KILL);
        }
        for task in &self.drains {
            task.abort();
        }
    }
}
