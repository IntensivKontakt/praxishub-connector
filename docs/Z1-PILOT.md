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

### B) VDDS-media — Dokumentenablage 👀 beobachten + zurückmelden
Hier sind wir bewusst im Beobachten-Modus (Antwortprotokoll am Z1 noch zu verifizieren):
- [ ] Taucht **Praxishub in Z1 als VDDS-Modul** auf bzw. wird es aufgerufen? (z. B. über
      einen „Bilder/Dokumente holen"-Button am Patienten, oder in der Modul-/Geräteliste.)
- [ ] Test-Patienten in Z1 öffnen, die Praxishub-Aktion auslösen → **kommt im Connector der
      Patientenkontext an**? Wird ein **Test-PDF in die Z1-Akte** übernommen?
- 📸 **Bitte notieren/screenshotten:** WIE bietet Z1 das an (Button/Pfad/Menü)? Welche
  Meldung kommt? Genau das brauche ich, um die Dokument-Ablage fertig zu bauen.

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
- 📸 Connector-Status-Fenster (Cloud/KIM/PVS grün?).
- Antworten zu **B** (VDDS-media: Button/Pfad/Verhalten) und **C** (HKP erkannt?).
- Welche **KIM-Host/Ports** real galten.
- Beobachtete Fehlermeldungen (Screenshot/Text).

Mit diesen Infos schließe ich die VDDS-media-Dokumentenablage und (bei vorhandenem
EBZ-Sample) das HKP-Parsing/die Auto-Einbestellung ab.
