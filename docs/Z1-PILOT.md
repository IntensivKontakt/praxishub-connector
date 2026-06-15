# Z1-Pilot — Schritt-für-Schritt-Anleitung

Ziel: Den Connector v0.1.0 an einer echten Praxis mit **Z1.PRO** installieren,
einrichten und die drei Bausteine prüfen. Teile davon sind **beobachtend** (wir
lernen, wie Z1 sich verhält) — bitte notieren/screenshotten, was unten als
„👀 beobachten + zurückmelden" markiert ist.

---

## 0. Vorab bereitlegen (am besten heute)

- [ ] **Windows-PC in der Praxis**, der das **KIM-Clientmodul erreicht** — idealerweise der
      Rechner, auf dem Z1.PRO **und** das KIM-Clientmodul laufen (sonst KIM-Host kennen).
- [ ] **Verbindungscode** der Pilot-Praxis: im Praxishub-Dashboard →
      **Einstellungen → Praxis → „Praxishub Connector" → „Verbindungscode erzeugen"** →
      kopieren/notieren. (Muss das Konto der Pilot-Praxis sein!)
- [ ] **KIM-Postfach-Zugangsdaten** aus dem KIM-Clientmodul unter **„Kontoinformationen"**:
      Host (meist `127.0.0.1`), **POP3S-Port** (`995`, bei CGM oft `8995`), Benutzer
      (`…@….kim.telematik`), Passwort.
- [ ] **KIM-Clientmodul: „Nachrichten auf dem Server belassen" / Aufbewahrung aktivieren.**
      ⚠️ Kritisch — sonst löscht Z1 die EBZ-Mail, bevor der Connector sie mitlesen kann.
- [ ] **Admin-Rechte** am PC (nur für die einmalige PVS-Registrierung; Windows fragt per UAC).

---

## 1. Installieren

1. Den signierten Installer von der Release-Seite herunterladen:
   `Praxishub.Connector_0.1.0_x64-setup.exe`
   (https://github.com/IntensivKontakt/praxishub-connector/releases/tag/v0.1.0)
2. Ausführen. Da signiert (Herausgeber „IntensivKontakt GmbH & Co. KG") → **keine**
   „Unbekannter Herausgeber"-Warnung. Per-User-Installation, **kein Admin für die Installation**.
3. Der Connector startet und zeigt den **Einrichtungs-Assistenten**.

## 2. Einrichten (First-Run-Wizard)

4. **Schritt 1 „Praxishub verbinden":** Verbindungscode einfügen → **„Verbindung testen"**.
   → muss „verbunden" zeigen. (Sonst: Code/Internetzugang prüfen.)
5. **Schritt 2 „KIM-Postfach":** Host / POP3-Port / Benutzer / Passwort eintragen →
   **„KIM testen"**. → muss „OK · N Nachricht(en) im Postfach" zeigen.
   (Sonst: Port/Host/Aufbewahrung prüfen. Connector läuft am besten **auf demselben
   Rechner** wie das KIM-Clientmodul, dann Host = `127.0.0.1`.)
6. **„Fertig & starten"** → Dashboard. Die Karten **Cloud** und **KIM** sollten grün sein.

## 3. PVS-Registrierung (VDDS-media)

7. Im Connector-Fenster **„Bei PVS registrieren"** → **Windows-UAC bestätigen**
   (das ist der einzige Admin-Schritt). → trägt Praxishub in die `VDDS_MMI.INI` ein.
   Karte **„PVS-Anbindung"** → „registriert".

---

## 4. Prüfen / beobachten — das ist der eigentliche Pilot

### A) Cloud-Heartbeat ✅ erwartet
- Im Praxishub-Dashboard sollte der Connector als **aktiv / „zuletzt gesehen"** auftauchen.

### B) VDDS-media — Z1-Fähigkeiten erheben (KEIN „Holen"-Button suchen!)
**Wichtig vorab:** VDDS-media ist standardmäßig PVS-initiiert — Z1 hat **keinen** generischen
„Dokumente holen"-Button und baut auch keinen. Der von uns gewünschte Weg (jede unterschriebene
Anamnese landet **automatisch** in der Z1-Akte) läuft über **VDDS-media Stufe 6 (BVS→PVS-Push)**:
Der Connector ruft Z1s Import-Modul `MMOINFIMPORT` auf und übergibt das PDF — ohne Klick in Z1.
Damit das geht, müssen wir im Piloten nur **zwei Dinge aus Z1 herauslesen**:

**B1 — `VDDS_MMI.INI` kopieren (das ist der wichtigste Artefakt!).**
- [ ] Datei `VDDS_MMI.INI` im Windows-Verzeichnis (`C:\Windows\VDDS_MMI.INI`, ggf. auch
      `Virtual Store`) **komplett kopieren / screenshotten** und mir schicken.
- Entscheidend ist die **Z1/PVS-Sektion** — welche Module hat Z1 registriert?
  - `MMOINFIMPORT=` (PVS-Importmodul) → **vorhanden = Auto-Push möglich** ✅
  - `PATDATIMPORT=` (bekommt Patientenkontext beim Öffnen) → für die PATID-Zuordnung
  - `IDEXPORT=` / `PVSLIMIT=` / unterstützte Datei-/Objekt-Typen → schreibe alles auf, was dort steht
- Nach unserer „Bei PVS registrieren"-Aktion sollte **auch eine `PRAXISHUB…`-BVS-Sektion** drinstehen
  → bitte mit-kopieren (bestätigt, dass die Registrierung gegriffen hat).

**B2 — Eine normale Patienten-/Bild-Aktion in Z1 auslösen (PATID-Zuordnung testen).**
- [ ] In Z1 einen **Test-Patienten öffnen** und eine **Röntgen-/Bild-/Mediafunktion** starten,
      die normalerweise eine Fremdsoftware aufruft (so wie das Team es täglich macht).
- [ ] Danach in den Connector-Logs prüfen: **kommt ein VDDS-Aufruf mit Patientenkontext
      (Name/Geburtsdatum/PATID) an?** (Genau daraus bauen wir später die Patienten-Zuordnung.)
- 📸 Notieren: Über welchen Weg/Button ruft Z1 normalerweise Fremdmodule auf?

Mit `VDDS_MMI.INI` + einem beobachteten PATDATIMPORT-Aufruf weiß ich sicher, ob der Auto-Push
direkt funktioniert oder ob wir einen Fallback (CGM-PRAXISARCHIV-Import) brauchen.

### C) KIM/EBZ — HKP-Erkennung 👀
- [ ] Liegt/trifft ein **genehmigter HKP (EBZ-Antwort)** im KIM-Postfach ein → im Praxishub
      sollte eine **Aufgabe „Genehmigter HKP eingegangen – Patient einbestellen"** erscheinen.
      (Die volle Auto-Einbestellung kommt später; jetzt zählt nur: **wird er erkannt?**)
- [ ] Falls aktuell kein echter EBZ vorliegt: notieren, ob welche im Postfach sind / wann der
      nächste erwartet wird (dann später nachprüfen).

---

## 5. Wenn etwas nicht läuft (Fehlersichtbarkeit)
- Das **Connector-Fenster** zeigt pro Baustein Status + letzten Fehler.
- Verstummt der Connector, alarmiert der **Backend-Watchdog** automatisch.
- **Logs** lokal: `%APPDATA%\ai.praxishub.connector\…\logs`.
- „KIM testen" / „Cloud testen" im Fenster zum Eingrenzen nutzen.

## 6. Bitte zurückmelden
- 📄 **`VDDS_MMI.INI` (komplett)** — der wichtigste Artefakt (siehe B1).
- 📸 Connector-Status-Fenster (Cloud/KIM/PVS grün?).
- Zu **B2**: kam ein VDDS-Aufruf mit Patientenkontext im Connector-Log an? Wie ruft Z1 Fremdmodule auf?
- Zu **C**: HKP/EBZ erkannt? (oder: liegen welche im Postfach / wann kommt der nächste?)
- Welche **KIM-Host/Ports** real galten.
- Beobachtete Fehlermeldungen (Screenshot/Text).

Mit `VDDS_MMI.INI` + einem beobachteten PATDATIMPORT-Aufruf entscheide ich, ob der
**Auto-Push (Stufe 6, `MMOINFIMPORT`)** direkt baubar ist, und schließe dann die automatische
Dokumentenablage ab. Bei vorhandenem EBZ-Sample folgt HKP-Parsing/Auto-Einbestellung.
