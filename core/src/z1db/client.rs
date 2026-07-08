//! Verbindung zur Z1-SQL-Server-Instanz (TDS via `tiberius`) + gemeinsame Helfer.
//!
//! Named-Instance-Auflösung (`srv-fs\z1`) läuft über den SQL Browser
//! (Feature `sql-browser-tokio`). TLS über `native-tls`; bei selbstsigniertem
//! Serverzertifikat `trust_cert()`.

use crate::error::{ConnectorError, Result};
use chrono::Local;
use tiberius::{AuthMethod, Client, Config, SqlBrowser};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

/// Laufende Z1-DB-Verbindung. Dünner Wrapper um den tiberius-Client.
pub struct Z1Connection {
    pub(crate) client: Client<Compat<TcpStream>>,
}

fn z1<E: std::fmt::Display>(ctx: &str) -> impl Fn(E) -> ConnectorError + '_ {
    move |e| ConnectorError::Z1Db(format!("{ctx}: {e}"))
}

/// Baut eine Verbindung auf. `server` = `host\instanz` (Named Instance) oder `host`.
pub async fn connect(
    server: &str,
    database: &str,
    user: &str,
    password: &str,
    trust_cert: bool,
) -> Result<Z1Connection> {
    let mut config = Config::new();
    let named = server.split_once('\\');
    match named {
        Some((host, instance)) => {
            config.host(host);
            config.instance_name(instance);
        }
        None => {
            config.host(server);
            config.port(1433);
        }
    }
    config.database(database);
    config.authentication(AuthMethod::sql_server(user, password));
    if trust_cert {
        config.trust_cert();
    }

    // Named Instance → Portauflösung über SQL Browser (connect_named).
    let tcp = if named.is_some() {
        TcpStream::connect_named(&config)
            .await
            .map_err(z1("Verbindung (SQL Browser)"))?
    } else {
        TcpStream::connect(config.get_addr())
            .await
            .map_err(z1("Verbindung"))?
    };
    tcp.set_nodelay(true).ok();

    let client = Client::connect(config, tcp.compat_write())
        .await
        .map_err(z1("Login"))?;
    Ok(Z1Connection { client })
}

impl Z1Connection {
    /// Verbindungs-/Auth-Check. Gibt die SQL-Server-Version zurück.
    pub async fn ping(&mut self) -> Result<String> {
        let row = self
            .client
            .query("SELECT @@VERSION", &[])
            .await
            .map_err(z1("Ping"))?
            .into_row()
            .await
            .map_err(z1("Ping"))?;
        Ok(row
            .and_then(|r| r.get::<&str, _>(0).map(str::to_string))
            .unwrap_or_default())
    }

    /// Liest genau einen String-Skalar (oder `None`, wenn keine Zeile/NULL).
    pub(crate) async fn scalar_string(
        &mut self,
        sql: &str,
        params: &[&dyn tiberius::ToSql],
    ) -> Result<Option<String>> {
        let row = self
            .client
            .query(sql, params)
            .await
            .map_err(z1("Query"))?
            .into_row()
            .await
            .map_err(z1("Query"))?;
        Ok(row.and_then(|r| r.get::<&str, _>(0).map(str::to_string)))
    }

    /// Liest genau einen i32-Skalar (0, wenn keine Zeile/NULL).
    pub(crate) async fn scalar_i32(
        &mut self,
        sql: &str,
        params: &[&dyn tiberius::ToSql],
    ) -> Result<i32> {
        let row = self
            .client
            .query(sql, params)
            .await
            .map_err(z1("Query"))?
            .into_row()
            .await
            .map_err(z1("Query"))?;
        Ok(row.and_then(|r| r.get::<i32, _>(0)).unwrap_or(0))
    }

    /// Führt eine schreibende Anweisung aus und verlangt exakt `expected` Zeilen.
    pub(crate) async fn exec_expect(
        &mut self,
        sql: &str,
        params: &[&dyn tiberius::ToSql],
        expected: u64,
    ) -> Result<()> {
        let res = self.client.execute(sql, params).await.map_err(z1("Exec"))?;
        let affected: u64 = res.rows_affected().iter().sum();
        if affected != expected {
            return Err(ConnectorError::Z1Db(format!(
                "Erwartet {expected} betroffene Zeile(n), waren {affected} — abgebrochen"
            )));
        }
        Ok(())
    }

    /// Liest alle Ergebniszeilen einer Abfrage (Ergebnismenge klein halten).
    pub(crate) async fn rows(
        &mut self,
        sql: &str,
        params: &[&dyn tiberius::ToSql],
    ) -> Result<Vec<tiberius::Row>> {
        self.client
            .query(sql, params)
            .await
            .map_err(z1("Query"))?
            .into_first_result()
            .await
            .map_err(z1("Query"))
    }

    /// Liest genau eine Zeile (oder `None`).
    pub(crate) async fn one_row(
        &mut self,
        sql: &str,
        params: &[&dyn tiberius::ToSql],
    ) -> Result<Option<tiberius::Row>> {
        self.client
            .query(sql, params)
            .await
            .map_err(z1("Query"))?
            .into_row()
            .await
            .map_err(z1("Query"))
    }

    /// Führt eine einfache (parameterlose) Anweisung aus — z. B. `BEGIN TRAN`.
    pub(crate) async fn simple(&mut self, sql: &str) -> Result<()> {
        self.client
            .simple_query(sql)
            .await
            .map_err(z1("Exec"))?
            .into_results()
            .await
            .map_err(z1("Exec"))?;
        Ok(())
    }
}

/// Erzeugt einen Z1-kompatiblen `RINFO`-Stempel: 17-stelliger Zeitstempel
/// (`JJJJMMTTHHMMSSmmm`) + unveränderter Rest eines bestehenden RINFO. Ohne
/// Vorlage ein Praxishub-Default-Suffix (`phb`), damit unsere Schreibzugriffe im
/// Replikations-Journal identifizierbar sind. Ergebnis auf 34 Zeichen begrenzt.
pub fn fresh_rinfo(existing: Option<&str>) -> String {
    let ts = Local::now().format("%Y%m%d%H%M%S%3f").to_string(); // 17 Zeichen
    let suffix = existing
        .filter(|r| r.len() >= 17)
        .map(|r| r[17..].to_string())
        .unwrap_or_else(|| " phb   1  0 291".to_string());
    let mut s = format!("{ts}{suffix}");
    if s.len() > 34 {
        s.truncate(34);
    }
    s
}

/// Rechtsbündig mit Leerzeichen auffüllen (Z1-Feldformat für `PATNR`/`LFD…`).
pub fn pad_left(value: &str, width: usize) -> String {
    let v = value.trim();
    if v.len() >= width {
        v.to_string()
    } else {
        format!("{}{}", " ".repeat(width - v.len()), v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_left_rechtsbuendig() {
        assert_eq!(pad_left("16006", 10), "     16006");
        assert_eq!(pad_left("0", 4), "   0");
        assert_eq!(pad_left("  42 ", 4), "  42"); // trimmt zuerst
        assert_eq!(pad_left("1234567890", 4), "1234567890"); // länger als Breite → unverändert
    }

    #[test]
    fn rinfo_uebernimmt_suffix_und_frischen_zeitstempel() {
        let old = "20260512044323573 54iiz  2 28 111";
        let r = fresh_rinfo(Some(old));
        // Suffix (ab Position 17) bleibt erhalten …
        assert_eq!(&r[17..], " 54iiz  2 28 111");
        // … der 17-stellige Zeitstempel ist neu (nur Ziffern) und ungleich alt.
        assert!(r[..17].chars().all(|c| c.is_ascii_digit()));
        assert_ne!(&r[..17], &old[..17]);
        assert!(r.len() <= 34);
    }

    #[test]
    fn rinfo_default_suffix_ohne_vorlage() {
        let r = fresh_rinfo(None);
        assert!(r.len() >= 17 && r.len() <= 34);
        assert!(r[..17].chars().all(|c| c.is_ascii_digit()));
    }
}
