//! Einmalige Einrichtung: Aus **temporär** eingegebenen Admin-Zugangsdaten einen
//! dedizierten **Read-only-Login** (`db_datareader`) anlegen. Die Admin-Daten
//! werden NIE gespeichert — nur der erzeugte Read-only-Login landet (DPAPI-
//! geschützt) in der Config. Siehe `docs/Z1-DATABASE.md` Abschnitt 1.

use crate::error::{ConnectorError, Result};
use crate::z1db::client::connect;

/// Nur `[A-Za-z0-9_]` als Login-Name zulassen (Identifier kann nicht als
/// Parameter gebunden werden → Injection-Schutz per Whitelist).
fn valid_identifier(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Legt (idempotent) einen Read-only-Login an: `CREATE LOGIN` + `CREATE USER` +
/// `db_datareader` auf der Ziel-DB. Nutzt die Admin-Verbindung nur transient.
pub async fn create_readonly_login(
    server: &str,
    database: &str,
    admin_user: &str,
    admin_password: &str,
    ro_user: &str,
    ro_password: &str,
    trust_cert: bool,
) -> Result<()> {
    if !valid_identifier(ro_user) {
        return Err(ConnectorError::Z1Db(
            "Ungültiger Read-only-Benutzername (nur A–Z, 0–9, _)".into(),
        ));
    }
    if !valid_identifier(database) {
        return Err(ConnectorError::Z1Db("Ungültiger Datenbankname".into()));
    }
    // CREATE LOGIN/USER akzeptieren keine gebundenen Parameter → Passwort inline,
    // einfache Anführungszeichen verdoppeln.
    let ro_pw = ro_password.replace('\'', "''");

    let mut admin = connect(server, "master", admin_user, admin_password, trust_cert).await?;

    admin
        .simple(&format!(
            "IF NOT EXISTS (SELECT 1 FROM sys.server_principals WHERE name = N'{ro_user}') \
             CREATE LOGIN [{ro_user}] WITH PASSWORD = N'{ro_pw}', CHECK_POLICY = OFF, \
             DEFAULT_DATABASE = [{database}]"
        ))
        .await?;
    admin.simple(&format!("USE [{database}]")).await?;
    admin
        .simple(&format!(
            "IF NOT EXISTS (SELECT 1 FROM sys.database_principals WHERE name = N'{ro_user}') \
             CREATE USER [{ro_user}] FOR LOGIN [{ro_user}]"
        ))
        .await?;
    admin
        .simple(&format!(
            "ALTER ROLE [db_datareader] ADD MEMBER [{ro_user}]"
        ))
        .await?;
    Ok(())
}
