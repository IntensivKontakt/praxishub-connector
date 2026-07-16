# Z1-Datenbank — Schema & lokale Entwicklungs-Kopie

Dieser Ordner enthält **nur PHI-freie** Artefakte: das Schema und Skripte. **Echte
Patientendaten werden NIEMALS committet** (DSGVO / besondere Kategorien
personenbezogener Daten) — sie bleiben ausschließlich lokal in der Praxisumgebung.

## Inhalt

| Datei | Inhalt | Committet? |
|---|---|---|
| `z1-schema.sql` | Vollständiges Schema aller 136 Tabellen (Spalten/Typen/PK/Indizes), aus der Live-DB generiert. Keine Daten. | ✅ ja |
| `make-local-dev-copy.ps1` | Erzeugt eine **lokale** Voll-Kopie (`.bak`) der Live-DB für die Entwicklung. Schreibt nur lokal. | ✅ ja (Skript) |
| `*.bak`, `local-data/`, `*-data*.sql` | Die eigentliche Daten-Kopie. | ❌ **nie** (`.gitignore`) |

Feld-/Beziehungssemantik: [../docs/Z1-SCHEMA.md](../docs/Z1-SCHEMA.md).
PVS-Architektur: [../docs/PVS-ARCHITEKTUR.md](../docs/PVS-ARCHITEKTUR.md).

## Warum keine Daten im Repo

Die Live-DB enthält Gesundheitsdaten echter Patienten (Namen, Versichertennummern,
Befunde, KIM-Nachrichten, Freitext mit PII). Leichte Pseudonymisierung (Name/Geburts-
datum ändern) macht das **nicht** repo-sicher: Behandlungshistorie ist selbst
identifizierend, die Versichertennummer bleibt ein direkter Identifikator, Adressen
liegen denormalisiert in vielen `*HIT`/`BILL`/`CASH`-Snapshots, und Freitext lässt sich
nicht zuverlässig scrubben. Git ist zudem unwiderruflich (History, Clones, GitHub).
→ **Die reale Daten-Kopie bleibt lokal**; das Repo bekommt nur Struktur + Skripte.

## Lokale Dev-Kopie erstellen (voll, mit echten Langzeit-Zusammenhängen)

Ziel: eine **lokale** DB, gegen die wir entwickeln, ohne die Produktiv-DB anzufassen.

**Voraussetzungen (einmalig):**
- Auf dem Dev-/Praxisrechner eine **SQL Server Developer/Express Edition** installieren
  (kostenlos) — das ist die lokale Ziel-Instanz. Die Produktiv-Instanz `srv-fs\z1`
  bleibt unangetastet.

**Schritt 1 — Backup ziehen** (nur Lesen der Live-DB, `COPY_ONLY` = stört die
CGM-Backup-Kette nicht). **Bitte außerhalb der Sprechzeiten** ausführen (IO-Last):

```powershell
powershell -ExecutionPolicy Bypass -File db\make-local-dev-copy.ps1 `
    -BackupDir "\\srv-fs\Backup\z1dev"   # ein für den SQL-Dienst schreibbarer Pfad
```

Das Skript erzeugt `Z1_dev_<datum>.bak` (komprimiert, mit CHECKSUM). Datei danach auf
den Dev-Rechner in einen **gitignorierten** Ordner kopieren (z. B. `db\local-data\`).

**Schritt 2 — In die lokale Instanz restoren** (auf der lokalen Dev-Instanz, dort hat
man selbst die nötigen Rechte):

```sql
RESTORE FILELISTONLY FROM DISK = 'C:\pfad\Z1_dev_<datum>.bak';  -- logische Namen ermitteln
RESTORE DATABASE Z1_DEV FROM DISK = 'C:\pfad\Z1_dev_<datum>.bak'
  WITH MOVE 'Z1'     TO 'C:\Z1DEV\Z1_DEV.mdf',
       MOVE 'Z1_log' TO 'C:\Z1DEV\Z1_DEV_log.ldf',
       REPLACE, RECOVERY;
```

**Schritt 3 — Connector/Tools auf die Dev-DB zeigen:** in der Config
`z1_db_server` = lokale Instanz, `z1_db_database` = `Z1_DEV`. Fertig — Entwicklung
läuft gegen die lokale Voll-Kopie.

> **Für eine echte Voll-Kopie des PVS** zusätzlich `CGMArchive` (PraxisArchiv-Blobs)
> gleich mitsichern: `-IncludeArchive` (siehe Skript). `z1trigger` (Replikations-
> journal) wird i. d. R. nicht benötigt.

## Alternative: Restore direkt als `Z1_DEV` auf `srv-fs`

Wenn die CGM-/IT-Administration eine `Z1_DEV`-Datenbank auf der bestehenden Instanz
anlegen kann (braucht `dbcreator`/`sysadmin` — unsere App-Logins haben das nicht),
ist das noch einfacher: nächtliches Z1-Backup als `Z1_DEV` restoren. Bleibt ebenfalls
in-perimeter, kein Repo.
