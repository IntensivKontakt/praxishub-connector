//! Z1-`ARCHIV`-Verlinkung importierter PraxisArchiv-Dokumente (Nelly-Parität).
//!
//! Der VDDS-Push legt das PDF zwar in PraxisArchiv ab, aber **niemand meldet es
//! an Z1**: Erst eine Zeile in der Tabelle `ARCHIV` macht das Dokument im
//! Z1-Karteireiter „Archiv" sichtbar. Nelly schrieb diese Zeile über die
//! Z1-interne `CZ1Archiv::AblageDocument`-API selbst (2749 Zeilen
//! „Anamnesebogen", 12/2024–07/2026); seit dem Umstieg auf den Connector fehlte
//! sie. Wir replizieren exakt Nellys Zeilenformat (verifiziert am Live-Z1,
//! Patient 18375): `EXTERNOBJEKTART=13` (VDDS-Typ „Anamnesebogen"), `BVS` leer,
//! `MMOID=archiv/fileID/seite`, `EXTERNARCHIVID=fileID+seite` (11-stellig
//! rechtsbündig), `PRAXISID/EXTERNPRAXISID=1`.
//!
//! Die PA-`MMOID` liefert der read-only **MMO-Info-Export** von ConVis
//! ([`crate::vdds::media::lookup_pa_mmoid`]) über unseren Korrelationsanker im
//! `COMMENT`-Feld (`Praxishub <doc-id>`).

use crate::config::ConnectorConfig;
use crate::error::{ConnectorError, Result};
use crate::z1db::client::{fresh_rinfo, pad_left};
use crate::z1db::{self, Z1Connection};
use chrono::Local;

/// Schreibt die `ARCHIV`-Indexzeile für ein frisch in PraxisArchiv abgelegtes
/// Dokument. Idempotent: existiert für `(PATNR, MMOID)` schon eine Zeile,
/// passiert nichts (`Ok(false)`). `Ok(true)` = Zeile eingefügt.
pub async fn link_pa_document(
    cfg: &ConnectorConfig,
    patnr: &str,
    pa_mmoid: &str,
    beschreibung: &str,
    externobjektart: u32,
) -> Result<bool> {
    let patnr = patnr.trim();
    if patnr.is_empty() || pa_mmoid.trim().is_empty() {
        return Err(ConnectorError::Z1Db("ARCHIV-Verlinkung ohne PATNR/MMOID".into()));
    }
    let mut conn = z1db::connect(
        &cfg.z1_db_server,
        &cfg.z1_db_database,
        &cfg.z1_db_write_user,
        &cfg.z1_db_write_password,
        cfg.z1_db_trust_cert,
    )
    .await?;
    insert_archiv_row(&mut conn, patnr, pa_mmoid, beschreibung, externobjektart).await
}

/// Kern-Insert (eigene Funktion, damit Verbindung/Transaktion testbar bleiben).
async fn insert_archiv_row(
    conn: &mut Z1Connection,
    patnr: &str,
    pa_mmoid: &str,
    beschreibung: &str,
    externobjektart: u32,
) -> Result<bool> {
    // Idempotenz: dieselbe PA-Datei nie doppelt indexieren.
    let existing = conn
        .scalar_i32(
            "SELECT COUNT(*) FROM ARCHIV WHERE LTRIM(RTRIM(PATNR)) = @P1 \
             AND LTRIM(RTRIM(ISNULL(MMOID,''))) = @P2",
            &[&patnr, &pa_mmoid.trim()],
        )
        .await?;
    if existing > 0 {
        return Ok(false);
    }

    // Nächste laufende Archiv-Nummer des Patienten (Nelly-Muster: max+1).
    let next = conn
        .scalar_i32(
            "SELECT ISNULL(MAX(CAST(LTRIM(RTRIM(LFDARCHIV)) AS INT)), 0) + 1 \
             FROM ARCHIV WHERE LTRIM(RTRIM(PATNR)) = @P1",
            &[&patnr],
        )
        .await?;

    let rinfo = fresh_rinfo(None);
    let patnr10 = pad_left(patnr, 10);
    let lfd4 = pad_left(&next.to_string(), 4);
    let datum = Local::now().format("%Y%m%d").to_string();
    let mut beschr = beschreibung.trim().to_string();
    if beschr.len() > 50 {
        let mut cut = 50;
        while !beschr.is_char_boundary(cut) {
            cut -= 1;
        }
        beschr.truncate(cut);
    }
    let eoa = externobjektart.to_string();
    let extern_archiv_id = pad_left(&externarchivid(pa_mmoid), 11);
    let praxisid = pad_left("1", 3);
    let externpraxisid = pad_left("1", 5);

    conn.exec_expect(
        "INSERT INTO ARCHIV \
         (RINFO,PATNR,LFDARCHIV,OBJEKTART,OBJEKTDATUM,OBJEKTBESCHREIBUNG,\
          EXTERNOBJEKTART,BVS,MMOID,EXTERNARCHIVID,PRAXISID,EXTERNPRAXISID,EPAUNIQUEID) \
         VALUES (@P1,@P2,@P3,'',@P4,@P5,@P6,'',@P7,@P8,@P9,@P10,'')",
        &[
            &rinfo,
            &patnr10,
            &lfd4,
            &datum,
            &beschr,
            &eoa,
            &pa_mmoid.trim(),
            &extern_archiv_id,
            &praxisid,
            &externpraxisid,
        ],
        1,
    )
    .await?;
    Ok(true)
}

/// `EXTERNARCHIVID` aus der PA-`MMOID` ableiten: `1/119649/1` → `1196491`
/// (Datei-ID + Seite, verifiziert an Nelly- UND Z1-eigenen Zeilen).
fn externarchivid(mmoid: &str) -> String {
    let mut parts = mmoid.trim().split('/');
    let (_archiv, file, page) = (parts.next(), parts.next(), parts.next());
    match (file, page) {
        (Some(f), Some(p)) => format!("{}{}", f.trim(), p.trim()),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn externarchivid_aus_mmoid() {
        // Nelly: MMOID 1/119649/1 → EXTERNARCHIVID 1196491
        assert_eq!(externarchivid("1/119649/1"), "1196491");
        // st (manuell): 1/119870/1 → 1198701
        assert_eq!(externarchivid("1/119870/1"), "1198701");
        assert_eq!(externarchivid("kaputt"), "");
        assert_eq!(externarchivid(""), "");
    }
}
