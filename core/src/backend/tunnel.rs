//! SSH tunnel through the system `ssh` client (macOS, Linux, Windows 10+ ship OpenSSH):
//! connection option `ssh=user@bastion[:port]` (plus `ssh_key=/path/to/key`) forwards a free
//! local port to the database host. Key or agent authentication only: ssh runs in batch mode.

use crate::{Backend, Bind, Canceller, Event, Result};
use anyhow::{Context, bail};
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub struct Tunnel {
    child: Child,
    pub port: u16,
}

/// `user@host[:port]` → (`user@host`, port).
fn target(spec: &str) -> (&str, Option<&str>) {
    match spec.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => (h, Some(p)),
        _ => (spec, None),
    }
}

impl Tunnel {
    pub fn open(spec: &str, key: Option<&str>, host: &str, port: u16) -> Result<Tunnel> {
        let local = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let (dest, ssh_port) = target(spec.trim());
        let mut cmd = Command::new("ssh");
        cmd.args(["-N", "-L", &format!("127.0.0.1:{local}:{host}:{port}")])
            .args(["-o", "ExitOnForwardFailure=yes", "-o", "BatchMode=yes", "-o", "ConnectTimeout=15", "-o", "StrictHostKeyChecking=accept-new", "-o", "ServerAliveInterval=30"]);
        if let Some(p) = ssh_port {
            cmd.args(["-p", p]);
        }
        if let Some(k) = key.filter(|k| !k.is_empty()) {
            cmd.args(["-i", k]);
        }
        cmd.arg(dest).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let child = cmd.spawn().context("starting ssh (is an OpenSSH client installed?)")?;
        let mut t = Tunnel { child, port: local };
        let start = Instant::now();
        loop {
            if TcpStream::connect_timeout(&([127, 0, 0, 1], local).into(), Duration::from_millis(200)).is_ok() {
                return Ok(t);
            }
            if let Some(status) = t.child.try_wait()? {
                let mut err = String::new();
                if let Some(mut e) = t.child.stderr.take() {
                    let _ = e.read_to_string(&mut err);
                }
                bail!("SSH tunnel to {dest} failed ({status}): {}\n(key or agent authentication only: set ssh_key=/path/to/key if needed)", err.trim());
            }
            if start.elapsed() > Duration::from_secs(20) {
                bail!("SSH tunnel to {dest} did not come up within 20 seconds");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A backend reached through a tunnel; the tunnel closes with the session.
pub struct Tunneled {
    pub inner: Box<dyn Backend>,
    pub _tunnel: Tunnel,
}

impl Backend for Tunneled {
    fn execute(&mut self, sql: &str, fetch: usize, sink: &mut dyn FnMut(Event) -> bool) -> Result<Option<u64>> {
        self.inner.execute(sql, fetch, sink)
    }
    fn commit(&mut self) -> Result<()> {
        self.inner.commit()
    }
    fn rollback(&mut self) -> Result<()> {
        self.inner.rollback()
    }
    fn set_autocommit(&mut self, on: bool) -> Result<()> {
        self.inner.set_autocommit(on)
    }
    fn enable_output(&mut self, on: bool) -> Result<()> {
        self.inner.enable_output(on)
    }
    fn take_output(&mut self) -> Result<Vec<String>> {
        self.inner.take_output()
    }
    fn canceller(&self) -> Option<Canceller> {
        self.inner.canceller()
    }
    fn execute_binds(&mut self, sql: &str, binds: &[Bind]) -> Result<Option<u64>> {
        self.inner.execute_binds(sql, binds)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ssh_target() {
        assert_eq!(super::target("me@bastion:2222"), ("me@bastion", Some("2222")));
        assert_eq!(super::target("me@bastion"), ("me@bastion", None));
    }
}
