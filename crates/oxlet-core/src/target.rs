use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde::{Deserialize, Serialize};

use crate::process::{self, output};

/// Where a project's files live and its commands run.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    #[default]
    Local,
    /// A WSL distro, reached through `wsl.exe` on Windows.
    Wsl { distro: String },
    /// A host the system `ssh` client reaches with a key: `[user@]host` or an `ssh://` URL.
    Ssh { host: String },
}

impl Target {
    /// The distro or host, for rows and messages; `None` for this machine.
    pub fn label(&self) -> Option<&str> {
        match self {
            Target::Local => None,
            Target::Wsl { distro } => Some(distro),
            Target::Ssh { host } => Some(host.strip_prefix("ssh://").unwrap_or(host)),
        }
    }

    /// Why a failed command never reached the target, when that is the reason it failed.
    pub fn unreachable(&self, code: Option<i32>, output: &str) -> Option<String> {
        let lines = || output.lines().map(str::trim).filter(|l| !l.is_empty());
        match self {
            Target::Local => None,
            // ssh exits 255 for its own failures: an unknown host, a refused key, no route.
            Target::Ssh { .. } if code == Some(255) => Some(format!(
                "Could not connect to {}: {}",
                self.label().unwrap_or_default(),
                lines().next_back().unwrap_or("ssh failed")
            )),
            // wsl.exe exits -1 for its own failures, such as a missing distro (255 seen from
            // inside WSL), and prints them on stdout.
            Target::Wsl { distro } if matches!(code, Some(-1 | 255)) => Some(format!(
                "WSL could not start {distro}: {}",
                lines().next().unwrap_or("wsl.exe failed")
            )),
            _ => None,
        }
    }

    /// [`Target::unreachable`] for a finished command.
    pub fn failure(&self, out: &Output) -> Option<String> {
        if out.status.success() {
            return None;
        }
        let stdout = match self {
            Target::Wsl { .. } => decode_wsl(&out.stdout),
            _ => String::new(),
        };
        let text = format!("{stdout}{}", String::from_utf8_lossy(&out.stderr));
        self.unreachable(out.status.code(), &text)
    }
}

/// A folder on a target: where a project's commands run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Place {
    pub target: Target,
    pub cwd: String,
}

/// The exit status of a script whose folder is missing (sysexits' EX_NOINPUT).
const NO_FOLDER: i32 = 66;

/// Agent CLIs usually live on the login PATH (`~/.local/bin` from `~/.profile`), so programs
/// run under the user's login shell. Shells outside the POSIX family take `-c` arguments
/// differently; those users get `sh`, which still reads `~/.profile`.
const LOGIN_SHELL: &str =
    "case ${SHELL##*/} in bash|zsh|ksh|dash|ash|sh) ;; *) SHELL=/bin/sh ;; esac";

impl Place {
    pub fn local(cwd: impl Into<String>) -> Place {
        Place {
            target: Target::Local,
            cwd: cwd.into(),
        }
    }

    pub fn new(target: Target, cwd: impl Into<String>) -> Place {
        Place {
            target,
            cwd: cwd.into(),
        }
    }

    /// The command that runs `program args` in this folder, and the script to write to its stdin
    /// when it goes through a target's client. With `watch`, the remote program and everything
    /// it started end when that stdin closes: killing an ssh client does not signal the command
    /// it started.
    pub fn command(
        &self,
        program: impl AsRef<OsStr>,
        args: &[impl AsRef<str>],
        watch: bool,
    ) -> (Command, Option<String>) {
        let client = match &self.target {
            Target::Local => {
                let mut cmd = process::command(program);
                cmd.args(args.iter().map(AsRef::as_ref))
                    .current_dir(&self.cwd);
                return (cmd, None);
            }
            Target::Wsl { distro } => wsl(distro),
            Target::Ssh { host } => ssh(host),
        };
        let mut words = vec![
            dir_word(&self.cwd),
            quote(&program.as_ref().to_string_lossy()),
        ];
        words.extend(args.iter().map(|a| quote(a.as_ref())));
        let run = format!(
            "\"$SHELL\" -lc 'cd -- \"$0\" || exit {NO_FOLDER}; exec \"$@\"' {}",
            words.join(" ")
        );
        let script = if watch {
            format!(
                "{{\n{LOGIN_SHELL}\nexec 3<&0\n{run} 3<&- &\np=$!\n{{ cat >/dev/null 2>&1; kill -TERM 0; }} <&3 &\nw=$!\nwait \"$p\"\ns=$?\nkill \"$w\" 2>/dev/null\nexit \"$s\"\n}}\n"
            )
        } else {
            format!("{LOGIN_SHELL}\nexec {run} </dev/null\n")
        };
        (client, Some(script))
    }

    /// This folder as a path the local file system reaches: itself on this machine, the
    /// `\\wsl.localhost` share of a distro on Windows.
    pub fn local_path(&self) -> Option<PathBuf> {
        match &self.target {
            Target::Local => Some(PathBuf::from(&self.cwd)),
            Target::Wsl { distro } if cfg!(windows) => Some(PathBuf::from(format!(
                r"\\wsl.localhost\{distro}{}",
                self.cwd.replace('/', r"\")
            ))),
            _ => None,
        }
    }
}

fn wsl(distro: &str) -> Command {
    let mut cmd = process::command("wsl.exe");
    cmd.args(["-d", distro, "-e", "sh"]);
    cmd
}

fn ssh(host: &str) -> Command {
    let mut cmd = process::command(std::env::var_os("OXLET_SSH").unwrap_or_else(|| "ssh".into()));
    cmd.args(["-T", "-o", "BatchMode=yes"]);
    if cfg!(unix) {
        // A dock refresh runs several git commands; one connection serves them all.
        let dir = crate::store::home();
        let _ = std::fs::create_dir_all(&dir);
        cmd.args(["-o", "ControlMaster=auto", "-o", "ControlPersist=60", "-o"])
            .arg(format!("ControlPath=\"{}\"", dir.join("ssh-%C").display()));
    }
    cmd.args(["--", host, "sh"]);
    cmd
}

/// Quotes `s` as one POSIX shell word.
pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A folder as a shell word, with a leading `~` as the target user's home.
fn dir_word(path: &str) -> String {
    match path.strip_prefix('~') {
        Some("") => "\"$HOME\"".into(),
        Some(rest) if rest.starts_with('/') => format!("\"$HOME\"{}", quote(rest)),
        _ => quote(path),
    }
}

/// The folder's absolute path on its target, or why a project can't live there.
pub fn resolve_folder(at: &Place) -> Result<String, String> {
    let out = process::run("pwd", &[] as &[&str], at).map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if out.status.success()
        && let Some(path) = stdout.lines().last()
    {
        return Ok(path.to_owned());
    }
    let place = at.target.label().unwrap_or("this machine");
    Err(match at.target.failure(&out) {
        Some(e) => e,
        None if out.status.code() == Some(NO_FOLDER) => {
            format!("{} is not a folder on {place}", at.cwd)
        }
        None => String::from_utf8_lossy(&out.stderr).trim().to_owned(),
    })
}

/// Where the target's login shell finds each of `binaries`.
pub fn find_programs(target: &Target, binaries: &[&str]) -> Result<Vec<Option<String>>, String> {
    let mut args = vec![
        "-c",
        r#"for b; do printf '%s=%s\n' "$b" "$(command -v "$b")"; done"#,
        "sh",
    ];
    args.extend(binaries);
    let text = output("sh", &args, &Place::new(target.clone(), "~"))?;
    Ok(binaries
        .iter()
        .map(|b| {
            text.lines()
                .filter_map(|l| l.split_once('='))
                .find(|(name, path)| name == b && path.starts_with('/'))
                .map(|(_, path)| path.to_owned())
        })
        .collect())
}

/// Creates `~/<relative>` on the target and returns its absolute path.
pub fn make_home_dir(target: &Target, relative: &str) -> Result<String, String> {
    let text = output(
        "sh",
        &["-c", r#"mkdir -p -- "$0" && cd -- "$0" && pwd"#, relative],
        &Place::new(target.clone(), "~"),
    )?;
    Ok(text.lines().last().unwrap_or_default().to_owned())
}

/// Each file's text, read on the target. A file larger than `cap` bytes, missing or not text
/// reads as empty.
pub fn read_texts(at: &Place, paths: &[String], cap: u64) -> Vec<String> {
    if let Some(root) = at.local_path() {
        return paths
            .iter()
            .map(|p| {
                let path = root.join(p);
                let small = std::fs::metadata(&path).is_ok_and(|m| m.len() <= cap);
                small
                    .then(|| std::fs::read_to_string(&path).ok())
                    .flatten()
                    .unwrap_or_default()
            })
            .collect();
    }
    // One process for every file. Each is `<length>\n<bytes>`, or `-` when it isn't a file;
    // C locale so `${#c}` counts bytes.
    let limit = (cap + 1).to_string();
    let mut args = vec![
        "-c",
        r#"LC_ALL=C; n=$1; shift; for f; do if [ -f "$f" ]; then c=$(head -c "$n" -- "$f"; echo x); c=${c%x}; printf '%s\n%s' "${#c}" "$c"; else echo -; fi; done"#,
        "sh",
        &limit,
    ];
    args.extend(paths.iter().map(String::as_str));
    let out = process::bytes("sh", &args, at).unwrap_or_default();
    parse_texts(&out, paths.len(), cap)
}

fn parse_texts(mut out: &[u8], count: usize, cap: u64) -> Vec<String> {
    let mut texts = Vec::with_capacity(count);
    while texts.len() < count
        && let Some(end) = out.iter().position(|&b| b == b'\n')
    {
        let size = std::str::from_utf8(&out[..end])
            .ok()
            .and_then(|n| n.parse::<usize>().ok());
        out = &out[end + 1..];
        texts.push(match size {
            Some(n) if n <= out.len() => {
                let (body, rest) = out.split_at(n);
                out = rest;
                if n as u64 > cap {
                    String::new()
                } else {
                    String::from_utf8(body.to_vec()).unwrap_or_default()
                }
            }
            _ => String::new(),
        });
    }
    texts.resize(count, String::new());
    texts
}

/// Reads a typed or picked folder. `\\wsl.localhost\<distro>\…` and `\\wsl$\<distro>\…` are a
/// WSL distro; `[user@]host:/path`, `host:~/path` and `ssh://[user@]host[:port]/path` an SSH
/// host; anything else a path on this machine.
pub fn parse_location(input: &str) -> (Target, String) {
    let input = input.trim();
    if let Some((distro, path)) = parse_wsl_share(input) {
        (Target::Wsl { distro }, path)
    } else if let Some((host, path)) = parse_ssh(input) {
        (Target::Ssh { host }, path)
    } else {
        (Target::Local, input.to_owned())
    }
}

fn parse_wsl_share(input: &str) -> Option<(String, String)> {
    let rest = input
        .strip_prefix(r"\\")
        .or_else(|| input.strip_prefix("//"))?;
    let mut parts = rest.split(['\\', '/']).filter(|p| !p.is_empty());
    let server = parts.next()?;
    if !server.eq_ignore_ascii_case("wsl.localhost") && !server.eq_ignore_ascii_case("wsl$") {
        return None;
    }
    let distro = parts.next()?.to_owned();
    let path: Vec<&str> = parts.collect();
    Some((distro, format!("/{}", path.join("/"))))
}

fn parse_ssh(input: &str) -> Option<(String, String)> {
    let usable = |host: &str| {
        !host.is_empty()
            && !host.starts_with('-')
            && !host.contains(|c: char| c.is_whitespace() || c == '/' || c == '\\')
    };
    if let Some(rest) = input.strip_prefix("ssh://") {
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = if authority.contains(':') {
            format!("ssh://{authority}")
        } else {
            authority.to_owned()
        };
        return usable(authority).then(|| (host, format!("/{path}")));
    }
    let (host, path) = input.split_once(':')?;
    // `C:\…` and `C:/…` are Windows drives.
    let drive = host.len() == 1 && host.chars().all(|c| c.is_ascii_alphabetic());
    (usable(host) && !drive && (path.starts_with('/') || path.starts_with('~')))
        .then(|| (host.to_owned(), path.to_owned()))
}

/// The `Host` names in an ssh config, without patterns.
pub fn ssh_config_hosts(config: &str) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    for line in config.lines() {
        let line = line.trim();
        let Some((keyword, value)) = line.split_once(|c: char| c.is_whitespace() || c == '=')
        else {
            continue;
        };
        if !keyword.eq_ignore_ascii_case("host") {
            continue;
        }
        for name in value
            .trim_start_matches(['=', ' ', '\t'])
            .split_whitespace()
        {
            if !name.contains(['*', '?', '!']) && !hosts.iter().any(|h| h == name) {
                hosts.push(name.to_owned());
            }
        }
    }
    hosts
}

/// The hosts named in `~/.ssh/config`.
pub fn ssh_hosts() -> Vec<String> {
    std::fs::read_to_string(crate::store::expand_home("~/.ssh/config"))
        .map(|c| ssh_config_hosts(&c))
        .unwrap_or_default()
}

/// wsl.exe writes UTF-16LE to a pipe unless `WSL_UTF8` is set, and UTF-8 then.
pub fn decode_wsl(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xFF, 0xFE]).unwrap_or(bytes);
    if bytes.len() >= 2 && bytes.iter().skip(1).step_by(2).any(|&b| b == 0) {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u16::from_le_bytes(pair))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// The WSL distros installed, when Oxlet runs on Windows.
pub fn wsl_distros() -> Vec<String> {
    if !cfg!(windows) {
        return Vec::new();
    }
    process::command("wsl.exe")
        .args(["-l", "-q"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_distros(&o.stdout))
        .unwrap_or_default()
}

fn parse_distros(bytes: &[u8]) -> Vec<String> {
    decode_wsl(bytes)
        .lines()
        .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\0'))
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The Linux build running inside WSL, where Windows programs are reachable.
pub fn inside_wsl() -> bool {
    cfg!(target_os = "linux")
        && (std::env::var_os("WSL_DISTRO_NAME").is_some()
            || Path::new("/proc/sys/fs/binfmt_misc/WSLInterop").exists())
}

/// Asks Windows for a folder from inside WSL, whose Linux desktop usually has no file chooser.
/// It starts in this distro's home; the answer is a Linux path.
pub fn windows_folder_dialog() -> Result<Option<String>, String> {
    let here = Place::local("/");
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    let start = output("wslpath", &["-w", &home], &here)?;
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; [Console]::OutputEncoding = [Text.Encoding]::UTF8; $d = New-Object System.Windows.Forms.FolderBrowserDialog; $d.SelectedPath = '{}'; $owner = New-Object System.Windows.Forms.Form -Property @{{TopMost = $true}}; if ($d.ShowDialog($owner) -eq 'OK') {{ $d.SelectedPath }}",
        start.trim().replace('\'', "''")
    );
    let picked = output(
        "powershell.exe",
        &["-NoProfile", "-STA", "-Command", &script],
        &here,
    )?;
    let picked = picked.trim();
    if picked.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        output("wslpath", &["-u", picked], &here)?.trim().to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_round_trips_through_sh() {
        let long = "x'y\"z$`\\\n".repeat(1300);
        let nasty = [
            "",
            "plain",
            "it's",
            "'",
            "''\\''",
            "a \"b\" $HOME `id` $(id) \\n \\",
            "line one\nline two\n\n",
            "tab\there; rm -rf / & echo |",
            "naïve — 日本語 🚀",
            "-n",
            long.as_str(),
        ];
        for s in nasty {
            let out = std::process::Command::new("sh")
                .args(["-c", &format!("printf '%s' {}", quote(s))])
                .output()
                .unwrap();
            assert_eq!(String::from_utf8(out.stdout).unwrap(), s);
        }
    }

    fn argv(cmd: &Command) -> Vec<String> {
        std::iter::once(cmd.get_program())
            .chain(cmd.get_args())
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn local_commands_run_directly_in_the_folder() {
        let (cmd, script) = Place::local("/repo").command("git", &["status"], true);
        assert_eq!(argv(&cmd), ["git", "status"]);
        assert_eq!(cmd.get_current_dir(), Some(Path::new("/repo")));
        assert!(script.is_none());
    }

    #[test]
    fn wsl_commands_keep_user_text_off_the_command_line() {
        let at = Place::new(
            Target::Wsl {
                distro: "Ubuntu".into(),
            },
            "/home/me/app",
        );
        let (cmd, script) = at.command("claude", &["-p", "it's \"done\""], false);
        assert_eq!(argv(&cmd), ["wsl.exe", "-d", "Ubuntu", "-e", "sh"]);
        let script = script.unwrap();
        assert!(script.contains(r#"'/home/me/app' 'claude' '-p' 'it'\''s "done"'"#));
        assert!(script.contains("exec \"$SHELL\" -lc"));
    }

    #[test]
    fn ssh_commands_fail_fast_and_name_the_host_once() {
        let at = Place::new(
            Target::Ssh {
                host: "me@devbox".into(),
            },
            "~/app",
        );
        let (cmd, script) = at.command("git", &["status"], true);
        let args = argv(&cmd);
        assert!(args.windows(2).any(|w| w == ["-o", "BatchMode=yes"]));
        assert_eq!(args[args.len() - 3..], ["--", "me@devbox", "sh"]);
        if cfg!(unix) {
            assert!(args.iter().any(|a| a.starts_with("ControlPath=")));
        }
        let script = script.unwrap();
        assert!(script.contains(r#""$HOME"'/app' 'git' 'status'"#));
        assert!(script.contains("kill -TERM 0"));
    }

    /// The scripts, run by a local `sh` the way a target's client runs them.
    #[cfg(unix)]
    #[test]
    fn scripts_run_the_program_in_the_folder_with_exact_arguments() {
        use std::io::{Read, Write};
        let dir = std::env::temp_dir().join(format!("oxlet script 'q' {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let at = Place::new(Target::Ssh { host: "h".into() }, dir.to_str().unwrap());
        let arg = "a 'b'\n$c `d` ü".repeat(500);
        for watch in [false, true] {
            let (_, script) = at.command(
                "sh",
                &["-c", "pwd; printf '%s' \"$1\"", "sh", arg.as_str()],
                watch,
            );
            // Its own process group: the watching script signals its group when stdin closes.
            let mut child = process::command("sh")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let mut stdin = child.stdin.take().unwrap();
            stdin.write_all(script.unwrap().as_bytes()).unwrap();
            let mut text = String::new();
            child
                .stdout
                .take()
                .unwrap()
                .read_to_string(&mut text)
                .unwrap();
            drop(stdin);
            assert!(child.wait().unwrap().success());
            assert_eq!(text, format!("{}\n{arg}", dir.display()));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn wsl_shares_name_a_distro_and_a_linux_path() {
        for share in [
            r"\\wsl.localhost\Ubuntu-24.04\home\me\app",
            r"\\wsl$\Ubuntu-24.04\home\me\app\",
            "//wsl.localhost/Ubuntu-24.04/home/me/app",
        ] {
            assert_eq!(
                parse_location(share),
                (
                    Target::Wsl {
                        distro: "Ubuntu-24.04".into()
                    },
                    "/home/me/app".into()
                )
            );
        }
        assert_eq!(
            parse_location(r"\\WSL.LOCALHOST\Debian").1,
            "/",
            "the distro's root"
        );
        assert_eq!(
            parse_location(r"\\fileserver\share\app"),
            (Target::Local, r"\\fileserver\share\app".into())
        );
    }

    #[test]
    fn ssh_locations_parse_and_drive_letters_do_not() {
        let ssh = |h: &str| Target::Ssh { host: h.into() };
        assert_eq!(
            parse_location("me@devbox:/srv/app"),
            (ssh("me@devbox"), "/srv/app".into())
        );
        assert_eq!(
            parse_location("ssh://me@devbox/srv/app"),
            (ssh("me@devbox"), "/srv/app".into())
        );
        assert_eq!(
            parse_location("ssh://me@devbox:2222/srv/app"),
            (ssh("ssh://me@devbox:2222"), "/srv/app".into())
        );
        assert_eq!(
            parse_location("devbox:~/app"),
            (ssh("devbox"), "~/app".into())
        );
        for local in [
            r"C:\work\app",
            "C:/work/app",
            "/home/me/a:b",
            "~/app",
            "-oProxyCommand=x:/a",
        ] {
            assert_eq!(parse_location(local).0, Target::Local, "{local}");
        }
    }

    #[test]
    fn ssh_config_hosts_skip_patterns() {
        let config = "Host *\n  ForwardAgent no\nHost devbox gpu-? !bad\n  HostName 10.0.0.2\nhost=build\nMatch host x\nHostName ignored\nHost devbox\n";
        assert_eq!(ssh_config_hosts(config), ["devbox", "build"]);
    }

    #[test]
    fn distro_lists_decode_from_utf16_and_utf8() {
        let utf16: Vec<u8> = "Ubuntu-24.04\r\nDebian\r\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(parse_distros(&utf16), ["Ubuntu-24.04", "Debian"]);
        let mut bom = vec![0xFF, 0xFE];
        bom.extend(&utf16);
        assert_eq!(parse_distros(&bom), ["Ubuntu-24.04", "Debian"]);
        assert_eq!(parse_distros(b"Ubuntu-26.04\r\n"), ["Ubuntu-26.04"]);
    }

    #[test]
    fn read_output_splits_files_and_drops_large_ones() {
        let out = b"5\nhello-\n0\n4\nbig!3\n\xff\xfe\n";
        assert_eq!(
            parse_texts(out, 5, 4),
            ["", "", "", "big!", ""],
            "5 bytes is over a cap of 4"
        );
        assert_eq!(parse_texts(out, 5, 8)[0], "hello");
    }

    #[test]
    fn connection_failures_read_as_such() {
        let ssh = Target::Ssh {
            host: "ssh://me@devbox:2222".into(),
        };
        assert_eq!(
            ssh.unreachable(
                Some(255),
                "Warning: added\nme@devbox: Permission denied (publickey).\n"
            )
            .as_deref(),
            Some("Could not connect to me@devbox:2222: me@devbox: Permission denied (publickey).")
        );
        assert_eq!(
            ssh.unreachable(Some(1), "fatal: not a git repository"),
            None
        );
        let wsl = Target::Wsl {
            distro: "Nope".into(),
        };
        assert_eq!(
            wsl.unreachable(
                Some(-1),
                "There is no distribution with the supplied name.\r\nError code: x"
            )
            .as_deref(),
            Some("WSL could not start Nope: There is no distribution with the supplied name.")
        );
    }
}
