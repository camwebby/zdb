//! SSH tunnels via the system `ssh`, so ~/.ssh/config, ProxyJump and the agent all apply.

use super::DbError;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

pub struct Tunnel {
    pub local_port: u16,
    _child: Child,
}

impl Tunnel {
    pub async fn open(target: &str, remote_host: &str, remote_port: u16) -> Result<Tunnel, DbError> {
        let local_port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .map_err(|e| DbError::msg(format!("no free local port for tunnel: {e}")))?;
        let forward = format!("127.0.0.1:{local_port}:{remote_host}:{remote_port}");
        let mut child = Command::new("ssh")
            .args(["-N", "-o", "ExitOnForwardFailure=yes", "-o", "BatchMode=yes", "-o", "ServerAliveInterval=30", "-L"])
            .arg(&forward)
            .arg(target)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| DbError::msg(format!("could not start ssh: {e}")))?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            if tokio::net::TcpStream::connect(("127.0.0.1", local_port)).await.is_ok() {
                return Ok(Tunnel { local_port, _child: child });
            }
            if let Ok(Some(status)) = child.try_wait() {
                let mut err = String::new();
                if let Some(mut e) = child.stderr.take() {
                    let _ = e.read_to_string(&mut err).await;
                }
                let err = err.trim();
                return Err(DbError::msg(format!(
                    "ssh tunnel to {target} failed ({status}){}{}",
                    if err.is_empty() { "" } else { ": " },
                    err.lines().last().unwrap_or("")
                )));
            }
            if tokio::time::Instant::now() > deadline {
                return Err(DbError::msg(format!("ssh tunnel to {target} timed out")));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
