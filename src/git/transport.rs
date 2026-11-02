//! System OpenSSH preserves the user's SSH config, agent and host-key checks.
//! Ordinary reads share a framed session on every client OS. History uses a
//! separate streaming process so an early search result can stop remote `git log`.
use crate::ssh_auth::{Askpass, SshHostVerification, SshSecret};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Output, Stdio},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(untagged)]
pub enum RepositoryLocation {
    Local(PathBuf),
    Ssh {
        path: String,
        #[serde(flatten)]
        options: SshOptions,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SshAuthentication {
    Password,
    #[default]
    Key,
    KeyWithPassphrase,
}

impl SshAuthentication {
    pub fn requires_secret(self) -> bool {
        self != Self::Key
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct SshOptions {
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default)]
    pub authentication: SshAuthentication,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_file: Option<PathBuf>,
}

impl From<PathBuf> for RepositoryLocation {
    fn from(path: PathBuf) -> Self {
        Self::Local(path)
    }
}
impl From<&PathBuf> for RepositoryLocation {
    fn from(path: &PathBuf) -> Self {
        Self::Local(path.clone())
    }
}
impl From<&RepositoryLocation> for RepositoryLocation {
    fn from(location: &RepositoryLocation) -> Self {
        location.clone()
    }
}

impl RepositoryLocation {
    pub fn validate(&self) -> Result<()> {
        if let Self::Ssh { options, path } = self {
            ensure!(
                !options.host.is_empty()
                    && !options.host.starts_with('-')
                    && options
                        .host
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-[]:".contains(&b)),
                "Enter an SSH hostname or IP address, without a username or command-line options"
            );
            if let Some(user) = &options.username {
                ensure!(
                    !user.is_empty()
                        && !user.starts_with('-')
                        && !user.contains(['\0', '\r', '\n', '@'])
                        && !user.chars().any(char::is_whitespace),
                    "Invalid SSH username"
                );
            }
            ensure!(
                options.port != Some(0),
                "SSH port must be between 1 and 65535"
            );
            ensure!(
                path.starts_with('/') && !path.contains('\0'),
                "Enter an absolute Linux repository path"
            );
            for file in [&options.config_file, &options.identity_file]
                .into_iter()
                .flatten()
            {
                ensure!(
                    file.is_file(),
                    "SSH configuration or private key file does not exist"
                );
            }
            ensure!(
                options.authentication != SshAuthentication::KeyWithPassphrase
                    || options.identity_file.is_some(),
                "Select a private key file when entering its passphrase"
            );
        }
        Ok(())
    }

    pub fn path(&self) -> &Path {
        match self {
            Self::Local(path) => path,
            Self::Ssh { path, .. } => Path::new(path),
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Ssh { .. })
    }

    pub fn name(&self) -> String {
        match self {
            Self::Local(path) => path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned(),
            Self::Ssh { path, .. } => path
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or(path)
                .to_owned(),
        }
    }
}

impl std::fmt::Display for RepositoryLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local(path) => write!(f, "{}", path.display()),
            Self::Ssh { options, path } => {
                if let Some(user) = &options.username {
                    write!(f, "{user}@")?;
                }
                write!(f, "{}", options.host)?;
                if let Some(port) = options.port {
                    write!(f, ":{port}")?;
                }
                write!(f, ":{path}")
            }
        }
    }
}

pub(super) fn quote(value: &str) -> Result<String> {
    ensure!(
        !value.contains('\0'),
        "NUL is not valid in an SSH command argument"
    );
    Ok(format!("'{}'", value.replace('\'', "'\\''")))
}

pub(super) struct GitCommand {
    inner: Command,
    remote: Option<Arc<SshConnection>>,
}

impl GitCommand {
    pub(super) fn local(inner: Command) -> Self {
        Self {
            inner,
            remote: None,
        }
    }
    pub(super) fn remote(mut self, remote: Option<Arc<SshConnection>>) -> Self {
        self.remote = remote;
        self
    }
    pub(super) fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.inner.arg(arg);
        self
    }
    pub(super) fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.inner.args(args);
        self
    }
    pub(super) fn stdin(&mut self, io: Stdio) -> &mut Self {
        self.inner.stdin(io);
        self
    }
    pub(super) fn stdout(&mut self, io: Stdio) -> &mut Self {
        self.inner.stdout(io);
        self
    }
    pub(super) fn stderr(&mut self, io: Stdio) -> &mut Self {
        self.inner.stderr(io);
        self
    }

    fn script(&self) -> Result<String> {
        let args = self
            .inner
            .get_args()
            .map(|arg| quote(arg.to_str().context("Remote Git paths must be UTF-8")?))
            .collect::<Result<Vec<_>>>()?;
        Ok(format!(
            "exec env GIT_TERMINAL_PROMPT=0 LC_ALL=C git {}",
            args.join(" ")
        ))
    }

    pub(super) fn output(&mut self) -> Result<Output> {
        match &self.remote {
            Some(remote) => remote.run(&self.script()?),
            None => self
                .inner
                .output()
                .context("Cannot start Git. Install Git and make it available on PATH."),
        }
    }

    pub(super) fn spawn(&mut self) -> Result<GitProcess> {
        match &self.remote {
            Some(remote) => remote.control.spawn(
                remote
                    .command(&self.script()?)?
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped()),
            ),
            None => GitProcess::spawn(&mut self.inner),
        }
    }
}

/// Cancellation is shared by every read in one SSH connection, including history streams.
#[derive(Clone, Debug, Default)]
pub struct SshConnectionControl(Arc<ConnectionControl>);

#[derive(Debug, Default)]
struct ConnectionControl {
    disconnected: AtomicBool,
    children: Mutex<Vec<Weak<Mutex<Child>>>>,
}

impl SshConnectionControl {
    pub fn disconnect(&self) {
        self.0.disconnected.store(true, Ordering::Release);
        for child in self
            .0
            .children
            .lock()
            .unwrap()
            .iter()
            .filter_map(Weak::upgrade)
        {
            let _ = child.lock().unwrap().kill();
        }
    }

    pub fn is_disconnected(&self) -> bool {
        self.0.disconnected.load(Ordering::Acquire)
    }

    fn spawn(&self, command: &mut Command) -> Result<GitProcess> {
        let mut children = self.0.children.lock().unwrap();
        ensure!(
            !self.is_disconnected(),
            "SSH connection has been disconnected"
        );
        let process = GitProcess::spawn(command)?;
        children.retain(|child| child.strong_count() > 0);
        children.push(Arc::downgrade(&process.child));
        Ok(process)
    }
}

#[derive(Debug)]
pub(super) struct GitProcess {
    child: Arc<Mutex<Child>>,
    pub(super) stdin: Option<ChildStdin>,
    pub(super) stdout: Option<ChildStdout>,
    errors: Option<JoinHandle<Vec<u8>>>,
}

impl GitProcess {
    fn spawn(command: &mut Command) -> Result<Self> {
        let mut child = command.spawn()?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let errors = child.stderr.take().map(|mut stderr| {
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                let _ = stderr.read_to_end(&mut bytes);
                bytes
            })
        });
        Ok(Self {
            child: Arc::new(Mutex::new(child)),
            stdin,
            stdout,
            errors,
        })
    }

    pub(super) fn kill(&self) -> std::io::Result<()> {
        self.child.lock().unwrap().kill()
    }

    fn wait(&self) -> std::io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.child.lock().unwrap().try_wait()? {
                return Ok(status);
            }
            // Leave the child accessible to disconnect while waiting for process teardown.
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    pub(super) fn wait_with_output(mut self) -> Result<Output> {
        let mut stdout = Vec::new();
        if let Some(mut stream) = self.stdout.take() {
            stream.read_to_end(&mut stdout)?;
        }
        let status = self.wait()?;
        let stderr = self
            .errors
            .take()
            .and_then(|thread| thread.join().ok())
            .unwrap_or_default();
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }
}

impl Drop for GitProcess {
    fn drop(&mut self) {
        let _ = self.kill();
        let _ = self.wait();
        if let Some(thread) = self.errors.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Debug)]
pub(super) struct SshConnection {
    options: SshOptions,
    askpass: Option<Askpass>,
    process: Mutex<Option<SshProcess>>,
    verify_host: bool,
    pub(super) control: SshConnectionControl,
}

impl SshConnection {
    pub(super) fn new(
        options: SshOptions,
        secret: Option<SshSecret>,
        verification: Option<SshHostVerification>,
        control: SshConnectionControl,
    ) -> Result<Self> {
        ensure!(
            options.authentication.requires_secret() == secret.is_some(),
            "This SSH connection requires a password or key passphrase"
        );
        let mut connection = Self {
            options,
            askpass: None,
            process: Mutex::new(None),
            verify_host: verification.is_some(),
            control,
        };
        let credential = secret
            .map(|secret| {
                connection
                    .credential_prompt()
                    .map(|prompt| (secret, prompt))
            })
            .transpose()?;
        if credential.is_some() || verification.is_some() {
            connection.askpass = Some(Askpass::start(credential, verification)?);
        }
        Ok(connection)
    }

    fn client_command(&self) -> Command {
        let mut command = Command::new("ssh");
        if let Some(config) = &self.options.config_file {
            command.arg("-F").arg(config);
        }
        command.args([
            "-T",
            "-a",
            "-x",
            "-o",
            if self.verify_host {
                "StrictHostKeyChecking=ask"
            } else {
                "StrictHostKeyChecking=yes"
            },
            "-o",
            "FingerprintHash=sha256",
            "-o",
            "UpdateHostKeys=no",
            "-o",
            "VerifyHostKeyDNS=no",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "ServerAliveInterval=10",
            "-o",
            "ServerAliveCountMax=2",
            "-o",
            "ClearAllForwardings=yes",
            "-o",
            "ControlMaster=no",
            "-o",
            "ControlPath=none",
            "-o",
            "StdinNull=no",
            "-o",
            "RemoteCommand=none",
        ]);
        if let Some(port) = self.options.port {
            command.arg("-p").arg(port.to_string());
        }
        if let Some(user) = &self.options.username {
            command.arg("-l").arg(user);
        }
        command.args([
            "-o",
            "NumberOfPasswordPrompts=1",
            "-o",
            "KbdInteractiveAuthentication=no",
        ]);
        match self.options.authentication {
            SshAuthentication::Password => {
                command.args([
                    "-o",
                    "PreferredAuthentications=password",
                    "-o",
                    "PubkeyAuthentication=no",
                    "-o",
                    "PasswordAuthentication=yes",
                ]);
            }
            SshAuthentication::Key | SshAuthentication::KeyWithPassphrase => {
                command.args([
                    "-o",
                    "PreferredAuthentications=publickey",
                    "-o",
                    "PasswordAuthentication=no",
                ]);
                if let Some(identity) = &self.options.identity_file {
                    command
                        .arg("-i")
                        .arg(identity)
                        .args(["-o", "IdentitiesOnly=yes"]);
                }
            }
        }
        if let Some(askpass) = &self.askpass {
            command.args(["-o", "BatchMode=no"]);
            askpass.configure(&mut command);
        } else {
            command.args(["-o", "BatchMode=yes"]);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        command
    }

    fn command(&self, script: &str) -> Result<Command> {
        let mut command = self.client_command();
        command
            .arg(&self.options.host)
            .arg(format!("sh -c {}", quote(script)?));
        Ok(command)
    }

    fn credential_prompt(&self) -> Result<String> {
        if self.options.authentication == SshAuthentication::KeyWithPassphrase {
            return Ok(format!(
                "Enter passphrase for key '{}':",
                self.options.identity_file.as_ref().unwrap().display()
            ));
        }
        // Resolve aliases and HostKeyAlias using OpenSSH itself, before asking
        // its helper to answer only the destination's password prompt.
        let output = self
            .client_command()
            .arg("-G")
            .arg(&self.options.host)
            .output()?;
        ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        let config = String::from_utf8(output.stdout)?;
        let value = |name: &str| {
            config
                .lines()
                .filter_map(|line| line.split_once(' '))
                .find_map(|(key, value)| (key == name).then_some(value))
        };
        let user = value("user").context("Missing SSH username")?;
        let host = value("hostkeyalias")
            .filter(|v| *v != "none")
            .or_else(|| value("hostname"))
            .context("Missing SSH hostname")?;
        Ok(format!("{user}@{host}'s password:"))
    }

    pub(super) fn run(&self, script: &str) -> Result<Output> {
        let mut process = self
            .process
            .lock()
            .map_err(|_| anyhow::anyhow!("SSH session lock failed"))?;
        ensure!(
            !self.control.is_disconnected(),
            "SSH connection has been disconnected"
        );
        if process.is_none() {
            *process = Some(SshProcess::start(
                self.command(include_str!("ssh_worker.sh"))?,
                &self.control,
            )?);
        }
        let result = process.as_mut().unwrap().run(script);
        if let Err(error) = result {
            let mut failed = process.take().unwrap();
            let diagnostic = failed.close();
            bail!(
                "SSH {}: {error:#}\n{}",
                self.options.host,
                diagnostic.trim()
            );
        }
        result
    }

    pub(super) fn read_file(&self, path: &str) -> Result<Option<Vec<u8>>> {
        let path = quote(path)?;
        let result = self.run(&format!(
            "if [ -e {path} ] || [ -L {path} ]; then cat -- {path}; else exit 44; fi"
        ))?;
        if result.status.code() == Some(44) {
            return Ok(None);
        }
        ensure!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr).trim()
        );
        Ok(Some(result.stdout))
    }
}

#[derive(Debug)]
struct SshProcess {
    child: GitProcess,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl SshProcess {
    fn start(mut command: Command, control: &SshConnectionControl) -> Result<Self> {
        let mut child = control
            .spawn(
                command
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped()),
            )
            .context("Cannot start SSH; install OpenSSH and make ssh available on PATH")?;
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Ok(Self {
            child,
            input,
            output,
        })
    }

    fn run(&mut self, script: &str) -> Result<Output> {
        writeln!(self.input, "{}", script.len())?;
        self.input.write_all(script.as_bytes())?;
        self.input.flush()?;
        let mut header = String::new();
        ensure!(
            self.output.read_line(&mut header)? > 0,
            "Connection closed; check the SSH host, key and known_hosts, then reconnect"
        );
        let fields = header
            .split_whitespace()
            .map(str::parse::<usize>)
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Unexpected SSH output before the response")?;
        ensure!(
            fields.len() == 3 && fields[0] <= 255,
            "Invalid SSH response"
        );
        let mut stdout = vec![0; fields[1]];
        let mut stderr = vec![0; fields[2]];
        self.output.read_exact(&mut stdout)?;
        self.output.read_exact(&mut stderr)?;
        #[cfg(unix)]
        let status = {
            use std::os::unix::process::ExitStatusExt;
            ExitStatus::from_raw((fields[0] as i32) << 8)
        };
        #[cfg(windows)]
        let status = {
            use std::os::windows::process::ExitStatusExt;
            ExitStatus::from_raw(fields[0] as u32)
        };
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }

    fn close(&mut self) -> String {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.child
            .errors
            .take()
            .and_then(|thread| thread.join().ok())
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default()
    }
}
impl Drop for SshProcess {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn disconnect_terminates_blocked_reads_and_prevents_late_processes() {
        let control = SshConnectionControl::default();
        let mut command = Command::new("sh");
        command
            .args(["-c", "printf ready; exec sleep 30"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut process = control.spawn(&mut command).unwrap();
        let mut ready = [0; 5];
        process
            .stdout
            .as_mut()
            .unwrap()
            .read_exact(&mut ready)
            .unwrap();
        assert_eq!(&ready, b"ready");
        let (sender, receiver) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || sender.send(process.wait_with_output()).unwrap());
        control.disconnect();
        let result = receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        assert!(!result.status.success());
        reader.join().unwrap();
        assert!(control.is_disconnected());
        assert!(control.spawn(&mut command).is_err());
        control.disconnect();
    }

    #[test]
    fn validates_remote_targets_without_interpreting_shell_syntax() {
        for host in ["server", "127.0.0.1", "2001:db8::1"] {
            RepositoryLocation::Ssh {
                options: SshOptions {
                    host: host.into(),
                    ..Default::default()
                },
                path: "/srv/a repo's $(literal)\nname".into(),
            }
            .validate()
            .unwrap();
        }
        for (host, path) in [
            ("-oProxyCommand=bad", "/srv/repo"),
            ("host;command", "/srv/repo"),
            ("host", "~/repo"),
            ("host", "/srv/a\0b"),
        ] {
            assert!(
                RepositoryLocation::Ssh {
                    options: SshOptions {
                        host: host.into(),
                        ..Default::default()
                    },
                    path: path.into(),
                }
                .validate()
                .is_err()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn framed_session_preserves_binary_output_status_and_quoted_arguments() {
        let mut command = Command::new("sh");
        command.arg("-c").arg(include_str!("ssh_worker.sh"));
        let mut session = SshProcess::start(command, &SshConnectionControl::default()).unwrap();
        let result = session
            .run("printf '\\000\\377\\n'; printf 'failure\\n' >&2; exit 7")
            .unwrap();
        assert_eq!(result.status.code(), Some(7));
        assert_eq!(result.stdout, b"\0\xff\n");
        assert_eq!(result.stderr, b"failure\n");
        let literal = "a'b; $(exit 9)\n中文";
        let result = session
            .run(&format!("printf '%s' {}", quote(literal).unwrap()))
            .unwrap();
        assert!(result.status.success());
        assert_eq!(result.stdout, literal.as_bytes());
        // An empty frame ends the worker and exercises its temporary-file cleanup.
        writeln!(session.input).unwrap();
        session.child.wait().unwrap();
    }
}
