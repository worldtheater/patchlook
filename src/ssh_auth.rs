//! In-memory credentials for the system SSH client's ASKPASS callback.
//! Only a loopback address and random capability token enter the child environment.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};
use zeroize::Zeroizing;

const ADDRESS: &str = "PATCHLOOK_ASKPASS_ADDRESS";
const TOKEN: &str = "PATCHLOOK_ASKPASS_TOKEN";
const IO_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_PROMPT_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostKey {
    pub host: String,
    pub algorithm: String,
    pub fingerprint: String,
}

impl HostKey {
    /// Only the unknown-key handshake prompt is eligible for approval. In
    /// particular, IP/key mismatch and generic permission prompts are rejected.
    fn parse(prompt: &str) -> Option<Self> {
        let prompt = prompt.trim_end();
        let (host, _) = prompt
            .strip_prefix("The authenticity of host '")?
            .split_once("' can't be established.")?;
        if !prompt.ends_with("(yes/no/[fingerprint])?") {
            return None;
        }
        let (algorithm, fingerprint) = prompt
            .lines()
            .find_map(|line| line.split_once(" key fingerprint is"))?;
        let fingerprint = fingerprint
            .trim_start_matches(':')
            .trim()
            .trim_end_matches('.');
        let hash = fingerprint.strip_prefix("SHA256:")?;
        if algorithm.is_empty()
            || !algorithm
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || hash.len() != 43
            || !hash
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"+/".contains(&b))
        {
            return None;
        }
        Some(Self {
            host: host.to_owned(),
            algorithm: algorithm.to_owned(),
            fingerprint: fingerprint.to_owned(),
        })
    }
}

/// The caller supplies a window-scoped decision route, never an automatic yes.
pub struct SshHostVerification {
    helper: PathBuf,
    confirm: Arc<dyn Fn(HostKey) -> bool + Send + Sync>,
}

impl SshHostVerification {
    pub fn new(helper: PathBuf, confirm: impl Fn(HostKey) -> bool + Send + Sync + 'static) -> Self {
        Self {
            helper,
            confirm: Arc::new(confirm),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Request {
    prompt: String,
    hint: Option<String>,
}

/// Deliberately has no Serialize or revealing Debug implementation.
pub struct SshSecret {
    value: Zeroizing<String>,
    helper: PathBuf,
}

impl SshSecret {
    pub fn new(value: String, helper: PathBuf) -> Result<Self> {
        let value = Zeroizing::new(value);
        Self::validate(&value)?;
        Ok(Self { value, helper })
    }

    pub fn validate(value: &str) -> Result<()> {
        ensure!(
            !value.is_empty() && value.len() < 1023 && !value.contains(['\0', '\r', '\n']),
            "SSH passwords and passphrases must be a nonempty single line shorter than 1023 bytes"
        );
        Ok(())
    }
}

pub(crate) struct Askpass {
    address: SocketAddr,
    token: String,
    helper: PathBuf,
    stopped: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Askpass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Askpass(<in-memory credential>)")
    }
}

impl Askpass {
    pub(crate) fn start(
        credential: Option<(SshSecret, String)>,
        verification: Option<SshHostVerification>,
    ) -> Result<Self> {
        let helper = credential
            .as_ref()
            .map(|(secret, _)| &secret.helper)
            .or_else(|| verification.as_ref().map(|v| &v.helper))
            .context("Missing ASKPASS helper")?
            .clone();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let address = listener.local_addr()?;
        let mut random = [0u8; 32];
        getrandom::fill(&mut random)
            .map_err(|error| anyhow::anyhow!("Cannot initialize SSH authentication: {error}"))?;
        let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let expected = token.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let done = stopped.clone();
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if done.load(Ordering::Acquire) {
                    break;
                }
                let Ok(mut stream) = stream else {
                    break;
                };
                let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
                let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
                let mut supplied = [0u8; 64];
                if stream.read_exact(&mut supplied).is_ok()
                    && supplied
                        .iter()
                        .zip(expected.as_bytes())
                        .fold(0, |diff, (a, b)| diff | (a ^ b))
                        == 0
                {
                    let _ = answer(&mut stream, credential.as_ref(), verification.as_ref());
                }
            }
        });
        Ok(Self {
            address,
            token,
            helper,
            stopped,
            thread: Some(thread),
        })
    }

    pub(crate) fn configure(&self, command: &mut Command) {
        command
            .env("SSH_ASKPASS", &self.helper)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("DISPLAY", "patchlook-askpass")
            .env(ADDRESS, self.address.to_string())
            .env(TOKEN, &self.token);
    }
}

impl Drop for Askpass {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        // Wake the blocking accept without a polling thread or credential file.
        let _ = TcpStream::connect_timeout(&self.address, IO_TIMEOUT);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Run before starting GPUI. OpenSSH invokes this executable with its prompt.
pub fn run_helper_if_requested() -> Option<i32> {
    std::env::var_os(ADDRESS)?;
    Some(if helper().is_ok() { 0 } else { 1 })
}

fn helper() -> Result<()> {
    let request = Request {
        prompt: std::env::args().nth(1).context("Missing SSH prompt")?,
        hint: std::env::var("SSH_ASKPASS_PROMPT").ok(),
    };
    let request = serde_json::to_vec(&request)?;
    ensure!(request.len() <= MAX_PROMPT_BYTES, "SSH prompt is too large");
    let address: SocketAddr = std::env::var(ADDRESS)?.parse()?;
    ensure!(
        address.ip() == Ipv4Addr::LOCALHOST,
        "Invalid SSH authentication endpoint"
    );
    let token = std::env::var(TOKEN)?;
    ensure!(
        token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid SSH authentication token"
    );
    let mut stream = TcpStream::connect_timeout(&address, IO_TIMEOUT)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    stream.write_all(token.as_bytes())?;
    stream.write_all(&(request.len() as u32).to_be_bytes())?;
    stream.write_all(&request)?;
    // A user decision has no deadline; closing the window drops its reply.
    stream.set_read_timeout(None)?;
    let mut value = Zeroizing::new(Vec::new());
    stream
        .take(1023)
        .read_to_end(&mut value)
        .context("SSH authentication channel closed")?;
    ensure!(!value.is_empty(), "SSH authentication unavailable");
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&value)?;
    stdout.flush()?;
    Ok(())
}

fn answer(
    stream: &mut TcpStream,
    credential: Option<&(SshSecret, String)>,
    verification: Option<&SshHostVerification>,
) -> Result<()> {
    let mut length = [0; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    ensure!(length <= MAX_PROMPT_BYTES, "SSH prompt is too large");
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes)?;
    let request: Request = serde_json::from_slice(&bytes)?;
    ensure!(
        request.hint.is_none(),
        "Unsupported SSH permission or notification prompt"
    );
    if let Some((secret, expected)) = credential
        && request.prompt.trim_end() == expected
    {
        stream.write_all(secret.value.as_bytes())?;
    } else if let Some(verification) = verification
        && let Some(key) = HostKey::parse(&request.prompt)
    {
        stream.write_all(if (verification.confirm)(key) {
            b"yes"
        } else {
            b"no"
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_unknown_sha256_host_keys_can_be_approved() {
        let fingerprint = format!("SHA256:{}", "A".repeat(43));
        for separator in [" ", ": "] {
            let prompt = format!(
                "The authenticity of host '[127.0.0.1]:2222 ([127.0.0.1]:2222)' can't be established.\nED25519 key fingerprint is{separator}{fingerprint}.\nAre you sure you want to continue connecting (yes/no/[fingerprint])? "
            );
            let key = HostKey::parse(&prompt).unwrap();
            assert_eq!(key.algorithm, "ED25519");
            assert_eq!(key.fingerprint, fingerprint);
            assert!(HostKey::parse(&prompt.replace("SHA256:", "MD5:")).is_none());
        }
        for prompt in [
            "Permission denied",
            "user@host's password:",
            "Please type 'yes' or 'no':",
            "Warning: the host key differs from the key for the IP address\nAre you sure you want to continue connecting (yes/no)?",
        ] {
            assert!(HostKey::parse(prompt).is_none());
        }
    }

    #[test]
    fn credential_channel_rejects_wrong_tokens_and_closes_with_its_session() {
        let secret = "not-for-environment-or-disk";
        let askpass = Askpass::start(
            Some((
                SshSecret::new(secret.into(), "helper".into()).unwrap(),
                "test prompt".into(),
            )),
            None,
        )
        .unwrap();
        let address = askpass.address;
        let mut command = Command::new("ssh");
        askpass.configure(&mut command);
        assert!(!format!("{command:?} {askpass:?}").contains(secret));
        for (token, expected) in [("0".repeat(64), ""), (askpass.token.clone(), secret)] {
            let mut stream = TcpStream::connect(address).unwrap();
            stream.write_all(token.as_bytes()).unwrap();
            if expected == secret {
                let request = serde_json::to_vec(&Request {
                    prompt: "test prompt".into(),
                    hint: None,
                })
                .unwrap();
                stream
                    .write_all(&(request.len() as u32).to_be_bytes())
                    .unwrap();
                stream.write_all(&request).unwrap();
            }
            let mut value = String::new();
            stream.read_to_string(&mut value).unwrap();
            assert_eq!(value, expected);
        }
        drop(askpass);
        assert!(TcpStream::connect(address).is_err());
    }
}
