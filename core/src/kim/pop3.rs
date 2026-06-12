//! Minimaler async POP3S-Client — **read-only by design**.
//!
//! Es gibt bewusst **kein `DELE`/`RSET`-Schreibkommando**: dieser Client kann
//! Nachrichten nicht löschen. So kann der Watcher dem PVS niemals eine EBZ-Mail
//! wegnehmen. Genutzt werden nur `STAT`, `UIDL`, `TOP`, `RETR`, `QUIT`.

use crate::error::{ConnectorError, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_native_tls::TlsStream;

pub struct Pop3Client {
    inner: BufReader<TlsStream<TcpStream>>,
}

/// (POP3-Nachrichtennummer, stabile UIDL)
pub type UidlEntry = (u32, String);

impl Pop3Client {
    /// Verbindet per implizitem TLS (POP3S, Port 995 / CGM 8995) und liest das
    /// Server-Greeting. `allow_invalid_cert` ist für localhost-Clientmodule
    /// gedacht, die selbstsignierte Zertifikate präsentieren.
    pub async fn connect(host: &str, port: u16, allow_invalid_cert: bool) -> Result<Self> {
        let tcp = TcpStream::connect((host, port)).await?;
        let mut builder = native_tls::TlsConnector::builder();
        if allow_invalid_cert {
            builder.danger_accept_invalid_certs(true);
            builder.danger_accept_invalid_hostnames(true);
        }
        let connector = tokio_native_tls::TlsConnector::from(
            builder.build().map_err(|e| ConnectorError::Tls(e.to_string()))?,
        );
        let tls = connector
            .connect(host, tcp)
            .await
            .map_err(|e| ConnectorError::Tls(e.to_string()))?;
        let mut client = Self { inner: BufReader::new(tls) };
        client.read_status().await?; // Greeting
        Ok(client)
    }

    pub async fn login(&mut self, user: &str, password: &str) -> Result<()> {
        self.command(&format!("USER {user}")).await?;
        self.command(&format!("PASS {password}")).await?;
        Ok(())
    }

    /// `STAT` → (Anzahl Nachrichten, Gesamtgröße in Bytes).
    pub async fn stat(&mut self) -> Result<(u32, u64)> {
        let line = self.command("STAT").await?;
        let mut parts = line.split_whitespace().skip(1); // "+OK" überspringen
        let count = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let size = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        Ok((count, size))
    }

    /// `UIDL` → alle (Nachrichtennummer, UIDL) auf dem Server.
    pub async fn uidl_all(&mut self) -> Result<Vec<UidlEntry>> {
        self.command("UIDL").await?;
        let body = self.read_multiline().await?;
        let mut out = Vec::new();
        for line in body.lines() {
            let mut p = line.split_whitespace();
            if let (Some(no), Some(uid)) = (p.next(), p.next()) {
                if let Ok(no) = no.parse::<u32>() {
                    out.push((no, uid.to_string()));
                }
            }
        }
        Ok(out)
    }

    /// `TOP msg n` → Header + n Body-Zeilen. Mit n=0 nur die Header — ideal, um
    /// die Dienstkennung zu prüfen, ohne die ganze Mail zu ziehen.
    pub async fn top(&mut self, msg_no: u32, body_lines: u32) -> Result<String> {
        self.command(&format!("TOP {msg_no} {body_lines}")).await?;
        self.read_multiline().await
    }

    /// `RETR msg` → vollständige RFC822-Nachricht.
    pub async fn retr(&mut self, msg_no: u32) -> Result<String> {
        self.command(&format!("RETR {msg_no}")).await?;
        self.read_multiline().await
    }

    pub async fn quit(&mut self) -> Result<()> {
        self.command("QUIT").await?;
        Ok(())
    }

    // --- intern ---

    async fn command(&mut self, cmd: &str) -> Result<String> {
        self.inner
            .get_mut()
            .write_all(format!("{cmd}\r\n").as_bytes())
            .await?;
        self.inner.get_mut().flush().await?;
        self.read_status().await
    }

    async fn read_line(&mut self) -> Result<String> {
        let mut line = String::new();
        let n = self.inner.read_line(&mut line).await?;
        if n == 0 {
            return Err(ConnectorError::Pop3("Verbindung vorzeitig geschlossen".into()));
        }
        Ok(line.trim_end_matches(['\r', '\n']).to_string())
    }

    /// Liest die Statuszeile (`+OK …` / `-ERR …`).
    async fn read_status(&mut self) -> Result<String> {
        let line = self.read_line().await?;
        if line.starts_with("+OK") {
            Ok(line)
        } else {
            Err(ConnectorError::Pop3(line))
        }
    }

    /// Liest eine Multiline-Antwort bis zur Terminierungszeile `.` und macht
    /// Dot-Stuffing rückgängig.
    async fn read_multiline(&mut self) -> Result<String> {
        let mut out = String::new();
        loop {
            let line = self.read_line().await?;
            if line == "." {
                break;
            }
            // Dot-Unstuffing: führender '.' einer Datenzeile entfernen.
            let unstuffed = line.strip_prefix('.').unwrap_or(&line);
            out.push_str(unstuffed);
            out.push('\n');
        }
        Ok(out)
    }
}
