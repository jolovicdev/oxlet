use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::provider::{self, AgentEvent, Protocol, ProviderId, TurnRequest};
use crate::target::{Place, Target};

/// A running child process. Dropping the handle leaves it running; `stop` ends it and everything
/// it started, since agent CLIs run their tools as child processes that hold the output open.
#[derive(Clone)]
pub struct Running {
    child: Arc<Mutex<Option<Child>>>,
    pid: u32,
    stopped: Arc<AtomicBool>,
    /// Set once the child is reaped, after which its pid may belong to another process.
    exited: Arc<AtomicBool>,
    /// Closes the stdin of a target's client, which ends the remote program.
    close: Option<Sender<()>>,
}

impl Running {
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(close) = &self.close {
            let _ = close.send(());
        }
        if !self.exited.load(Ordering::SeqCst) {
            kill_tree(self.pid);
        }
        if let Some(child) = self.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }

    pub fn was_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }
}

#[cfg(unix)]
fn kill_tree(pid: u32) {
    let _ = Command::new("kill")
        .args(["-TERM", "--", &format!("-{pid}")])
        .stderr(Stdio::null())
        .status();
}

#[cfg(windows)]
fn kill_tree(pid: u32) {
    let _ = command("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// A command that runs without a console window on Windows and in its own process group on Unix.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd
}

pub enum Line {
    Out(String),
    Err(String),
    Exit(Option<i32>),
}

/// Runs `program` at `at`, calling `sink` for each stdout and stderr line and once at exit.
pub fn spawn_lines(
    program: &Path,
    args: &[String],
    at: &Place,
    sink: impl Fn(Line) + Send + Sync + 'static,
) -> std::io::Result<Running> {
    let (mut cmd, script) = at.command(program, args, true);
    let mut child = cmd
        .stdin(if script.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdin = child.stdin.take();
    let pid = child.id();
    let shared = Arc::new(Mutex::new(Some(child)));
    let stopped = Arc::new(AtomicBool::new(false));
    let sink = Arc::new(sink);
    let close = match (script, stdin) {
        (Some(script), Some(mut stdin)) => {
            let (close, closed) = channel::<()>();
            // A thread, so a slow connection never blocks the caller. The pipe stays open until
            // a stop or the exit.
            thread::spawn(move || {
                let _ = stdin.write_all(script.as_bytes());
                let _ = closed.recv();
            });
            Some(close)
        }
        _ => None,
    };

    let err_sink = sink.clone();
    let err_reader = thread::spawn(move || {
        if let Some(err) = stderr {
            each_line(err, |line| err_sink(Line::Err(line)));
        }
    });

    let waiter = shared.clone();
    let exited = Arc::new(AtomicBool::new(false));
    let reaped = exited.clone();
    let closer = close.clone();
    thread::spawn(move || {
        if let Some(out) = stdout {
            each_line(out, |line| sink(Line::Out(line)));
        }
        let _ = err_reader.join();
        // Taken out of the lock before waiting: a program that closed its output can run on,
        // and `stop` needs the lock meanwhile.
        let child = waiter.lock().unwrap().take();
        let code = child.and_then(|mut c| c.wait().ok()).and_then(|s| s.code());
        reaped.store(true, Ordering::SeqCst);
        if let Some(closer) = closer {
            let _ = closer.send(());
        }
        sink(Line::Exit(code));
    });
    Ok(Running {
        child: shared,
        pid,
        stopped,
        exited,
        close,
    })
}

/// Calls `f` with each line of `from`. Bytes that are not UTF-8 are replaced, not the end of it.
fn each_line(from: impl Read, mut f: impl FnMut(String)) {
    let mut reader = BufReader::new(from);
    let mut line = Vec::new();
    while matches!(reader.read_until(b'\n', &mut line), Ok(n) if n > 0) {
        if line.ends_with(b"\n") {
            line.pop();
            if line.ends_with(b"\r") {
                line.pop();
            }
        }
        f(String::from_utf8_lossy(&line).into_owned());
        line.clear();
    }
}

/// Runs a shell command line the way a terminal would.
pub fn spawn_shell(
    command: &str,
    at: &Place,
    sink: impl Fn(Line) + Send + Sync + 'static,
) -> std::io::Result<Running> {
    let (shell, flag) = if at.target != Target::Local {
        ("sh".to_owned(), "-c")
    } else if cfg!(windows) {
        ("cmd".to_owned(), "/C")
    } else {
        (
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
            "-c",
        )
    };
    spawn_lines(
        Path::new(&shell),
        &[flag.to_owned(), command.to_owned()],
        at,
        sink,
    )
}

/// Runs one agent turn and reports its events. Exactly one `Done` is always delivered last.
pub fn spawn_turn(
    id: ProviderId,
    program: &Path,
    req: &TurnRequest,
    sink: impl Fn(AgentEvent) + Send + Sync + 'static,
) -> std::io::Result<Running> {
    let protocol = id.info().protocol;
    let args = provider::arguments(id, req);
    let state = Mutex::new(TurnState::default());
    let target = req.place.target.clone();
    spawn_lines(program, &args, &req.place, move |line| {
        let mut st = state.lock().unwrap();
        match line {
            Line::Out(text) if protocol == Protocol::Plain => {
                st.plain.push_str(&text);
                st.plain.push('\n');
            }
            Line::Out(text) => {
                for event in provider::parse_line(protocol, &text) {
                    if matches!(event, AgentEvent::Done { .. }) {
                        if st.done {
                            continue;
                        }
                        st.done = true;
                    }
                    sink(event);
                }
            }
            Line::Err(text) => {
                st.stderr.push_str(&text);
                st.stderr.push('\n');
            }
            Line::Exit(code) => {
                if !st.plain.trim().is_empty() {
                    sink(AgentEvent::Text(st.plain.trim_end().to_owned()));
                }
                if !st.done {
                    let error = (code != Some(0)).then(|| {
                        target
                            .unreachable(code, &st.stderr)
                            .or_else(|| tail(&st.stderr, 12))
                            .unwrap_or_else(|| format!("Exited with {code:?}"))
                    });
                    sink(AgentEvent::Done {
                        duration_ms: None,
                        error,
                    });
                }
            }
        }
    })
}

#[derive(Default)]
struct TurnState {
    plain: String,
    stderr: String,
    done: bool,
}

fn tail(text: &str, lines: usize) -> Option<String> {
    let all: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    (!all.is_empty()).then(|| all[all.len().saturating_sub(lines)..].join("\n"))
}

/// Runs a command to completion and returns its stdout, or its stderr as the error.
pub fn output(program: &str, args: &[&str], at: &Place) -> Result<String, String> {
    bytes(program, args, at).map(|out| String::from_utf8_lossy(&out).into_owned())
}

/// [`output`], with stdout as it came.
pub fn bytes(program: &str, args: &[&str], at: &Place) -> Result<Vec<u8>, String> {
    let out = run(program, args, at).map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        return Ok(out.stdout);
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(match at.target.failure(&out) {
        Some(e) => e,
        None if err.trim().is_empty() => format!("{program} exited with {}", out.status),
        None => err.trim().to_owned(),
    })
}

/// Runs a command to completion at `at`.
pub fn run(program: &str, args: &[&str], at: &Place) -> std::io::Result<Output> {
    let (mut cmd, script) = at.command(program, args, false);
    let Some(script) = script else {
        return cmd.stdin(Stdio::null()).output();
    };
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // The program starts only once the script is read whole, so this write never waits on it.
    // A client that could not connect has closed the pipe; its stderr says why.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(script.as_bytes());
    }
    child.wait_with_output()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    use crate::model::{AccessMode, ChatMode, Effort};

    #[test]
    fn shell_lines_arrive_in_order_then_exit() {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        spawn_shell(
            "echo one; echo two 1>&2; exit 3",
            &Place::local("/"),
            move |l| {
                let s = match l {
                    Line::Out(t) => format!("out:{t}"),
                    Line::Err(t) => format!("err:{t}"),
                    Line::Exit(c) => format!("exit:{c:?}"),
                };
                tx.lock().unwrap().send(s).unwrap();
            },
        )
        .unwrap();
        let got: Vec<String> = rx.iter().take(3).collect();
        assert!(got.contains(&"out:one".to_owned()) && got.contains(&"err:two".to_owned()));
        assert_eq!(got.last().unwrap(), "exit:Some(3)");
    }

    #[test]
    fn output_that_is_not_utf8_reads_through_to_the_exit() {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        spawn_shell("printf 'a\\377b\\nnext\\n'", &Place::local("/"), move |l| {
            if let Line::Out(t) | Line::Err(t) = l {
                tx.lock().unwrap().send(t).unwrap();
            }
        })
        .unwrap();
        let got: Vec<String> = rx.iter().take(2).collect();
        assert_eq!(got, ["a\u{FFFD}b", "next"]);
    }

    #[test]
    fn stopping_a_program_that_closed_its_output_ends_it_at_once() {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let running = spawn_shell(
            "exec sleep 30 >/dev/null 2>&1",
            &Place::local("/"),
            move |l| {
                if let Line::Exit(_) = l {
                    tx.lock().unwrap().send(()).unwrap();
                }
            },
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let start = std::time::Instant::now();
        running.stop();
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "stop waited for the program"
        );
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn a_failing_plain_turn_reports_stderr_once() {
        let dir = std::env::temp_dir().join(format!("oxlet-turn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("droid");
        std::fs::write(&script, "#!/bin/sh\necho partial\necho boom >&2\nexit 1\n").unwrap();
        std::process::Command::new("chmod")
            .args(["+x", script.to_str().unwrap()])
            .status()
            .unwrap();
        let req = TurnRequest {
            prompt: "x".into(),
            model: String::new(),
            effort: Effort::Low,
            access: AccessMode::ReadOnly,
            mode: ChatMode::Default,
            session: None,
            place: Place::local(dir.to_string_lossy()),
        };
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        spawn_turn(ProviderId::Droid, &script, &req, move |e| {
            tx.lock().unwrap().send(e).unwrap()
        })
        .unwrap();
        let a = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let b = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(a, AgentEvent::Text("partial".into()));
        assert_eq!(
            b,
            AgentEvent::Done {
                duration_ms: None,
                error: Some("boom".into())
            }
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
