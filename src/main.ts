import "./styles.css";
import { invoke } from "@tauri-apps/api/core";

// --- Typen spiegeln die Rust-Structs (commands.rs / config.rs / state.rs) ---
type Health = "ok" | "warn" | "err" | "unknown";

interface StatusSnapshot {
  version: string;
  vdds: { state: Health; detail: string };
  kim: { state: Health; detail: string };
  cloud: { state: Health; detail: string };
  last_hkp_at: string | null;
}

interface ConnectorConfig {
  praxishub_base_url: string;
  tenant_id: string;
  api_key: string;
  kim_host: string;
  kim_port: number;
  kim_user: string;
  kim_password: string;
  kim_poll_seconds: number;
  exchange_dir: string;
  // Z1-Datenbank (Lesen + strukturiertes Rückschreiben)
  z1_db_server: string;
  z1_db_database: string;
  z1_db_user: string;
  z1_db_password: string;
  z1_db_write_user: string;
  z1_db_write_password: string;
  z1_db_trust_cert: boolean;
  z1_hkp_lookback_months: number;
  z1_par_punktwert: number;
  // Rückschreib-Toggles
  writeback_contact: boolean;
  writeback_address: boolean;
  writeback_cave: boolean;
  writeback_anamnese: boolean;
  writeback_notes: boolean;
  writeback_new_patient: boolean;
  writeback_co_to_risk: boolean;
  writeback_archiv_link: boolean;
  pvs_file_invoices: boolean;
  // Praxis-Steuerung: nächtlicher Umsatz-/Leistungs-Aggregat-Sync (opt-in)
  z1_control_enabled: boolean;
  z1_control_hour: number;
  z1_control_months: number;
  z1_control_column_map: Record<string, string> | null;
}

let cfg: ConnectorConfig = blankConfig();

function blankConfig(): ConnectorConfig {
  return {
    praxishub_base_url: "https://api.praxishub.ai",
    tenant_id: "",
    api_key: "",
    kim_host: "127.0.0.1",
    kim_port: 995,
    kim_user: "",
    kim_password: "",
    kim_poll_seconds: 60,
    exchange_dir: "",
    z1_db_server: "",
    z1_db_database: "Z1",
    z1_db_user: "",
    z1_db_password: "",
    z1_db_write_user: "",
    z1_db_write_password: "",
    z1_db_trust_cert: true,
    z1_hkp_lookback_months: 24,
    z1_par_punktwert: 0,
    writeback_contact: false,
    writeback_address: false,
    writeback_cave: false,
    writeback_anamnese: false,
    writeback_notes: false,
    writeback_new_patient: false,
    writeback_co_to_risk: false,
    writeback_archiv_link: false,
    pvs_file_invoices: false,
    z1_control_enabled: false,
    z1_control_hour: 3,
    z1_control_months: 36,
    z1_control_column_map: null,
  };
}

const $ = <T extends HTMLElement = HTMLElement>(sel: string) =>
  document.querySelector(sel) as T;
const val = (id: string) => ($(`#${id}`) as HTMLInputElement)?.value.trim() ?? "";
const checked = (id: string) => ($(`#${id}`) as HTMLInputElement | null)?.checked ?? false;
const hasEl = (id: string) => !!($(`#${id}`) as HTMLElement | null);
function setChecked(id: string, v: boolean) {
  const el = $(`#${id}`) as HTMLInputElement | null;
  if (el) el.checked = v;
}

function toast(msg: string) {
  const t = $("#toast");
  if (!t) return;
  t.textContent = msg;
  t.classList.add("show");
  setTimeout(() => t.classList.remove("show"), 2600);
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T | null> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    console.warn(`invoke ${cmd} failed:`, e);
    return null;
  }
}

/** Eingerichtet, wenn Cloud- UND KIM-Zugang vorhanden sind. */
function isConfigured(c: ConnectorConfig): boolean {
  return !!(c.praxishub_base_url && c.api_key && c.kim_host && c.kim_user && c.kim_password);
}

/** Verbindungscode aus dem Dashboard: base64url(JSON {url, tenant, key}). */
function decodeConnectionCode(code: string): { url?: string; tenant?: string; key?: string } | null {
  try {
    const norm = code.trim().replace(/-/g, "+").replace(/_/g, "/");
    const json = JSON.parse(atob(norm));
    return { url: json.url, tenant: json.tenant, key: json.key };
  } catch {
    return null;
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// First-Run-Wizard
// ─────────────────────────────────────────────────────────────────────────────

function renderWizard() {
  $("#app").innerHTML = `
    <header><h1>Praxishub Connector</h1><span class="ver" id="ver"></span></header>
    <p class="sub">Ersteinrichtung – in zwei Schritten verbunden.</p>

    <div class="steps">
      <div class="step active" id="st1">1 · Praxishub verbinden</div>
      <div class="step" id="st2">2 · KIM-Postfach</div>
    </div>

    <section>
      <h2>1 · Praxishub verbinden</h2>
      <div class="field">
        <label>Verbindungscode aus dem Praxishub-Dashboard</label>
        <textarea id="code" placeholder="Code hier einfügen …"></textarea>
        <p class="hint">Dashboard → Einstellungen → Connector → „Verbindungscode erzeugen". Kein Abtippen von Schlüsseln nötig.</p>
      </div>
      <button id="apply_code">Code übernehmen</button>

      <details class="collapsible">
        <summary>Stattdessen manuell eingeben</summary>
        <div class="field" style="margin-top:10px"><label>Praxishub-URL</label><input id="praxishub_base_url" placeholder="https://api.praxishub.ai" /></div>
        <div class="row">
          <div class="field"><label>Tenant</label><input id="tenant_id" placeholder="praxis-xyz" /></div>
          <div class="field"><label>API-Key</label><input id="api_key" type="password" placeholder="wp_ext_…" /></div>
        </div>
      </details>

      <div class="actions"><button class="primary" id="test_cloud">Verbindung testen</button></div>
      <div class="result" id="cloud_result"></div>
    </section>

    <section>
      <h2>2 · KIM-Postfach</h2>
      <p class="sub" style="margin-bottom:12px">Zugangsdaten aus dem KIM-Clientmodul („Kontoinformationen").</p>
      <div class="row">
        <div class="field"><label>Host</label><input id="kim_host" value="127.0.0.1" /></div>
        <div class="field"><label>POP3-Port</label><input id="kim_port" value="995" /></div>
      </div>
      <div class="row">
        <div class="field"><label>Postfach (Benutzer)</label><input id="kim_user" placeholder="praxis@kim.telematik" /></div>
        <div class="field"><label>Passwort</label><input id="kim_password" type="password" /></div>
      </div>
      <div class="actions"><button id="test_kim">KIM testen</button></div>
      <div class="result" id="kim_result"></div>
    </section>

    <div class="wizard-foot">
      <span class="hint">Die PVS-Registrierung folgt danach im Hauptfenster.</span>
      <button class="primary" id="finish">Fertig & starten</button>
    </div>
    <div class="toast" id="toast"></div>
  `;

  prefill();
  $("#apply_code").addEventListener("click", onApplyCode);
  $("#test_cloud").addEventListener("click", onTestCloud);
  $("#test_kim").addEventListener("click", onTestKim);
  $("#finish").addEventListener("click", onFinish);
}

function prefill() {
  setIf("praxishub_base_url", cfg.praxishub_base_url);
  setIf("tenant_id", cfg.tenant_id);
  setIf("api_key", cfg.api_key);
  setIf("kim_host", cfg.kim_host);
  setIf("kim_port", String(cfg.kim_port || 995));
  setIf("kim_user", cfg.kim_user);
  setIf("kim_password", cfg.kim_password);
}
function setIf(id: string, v: string) {
  const el = $(`#${id}`) as HTMLInputElement | null;
  if (el && v) el.value = v;
}

function collectFromWizard(): ConnectorConfig {
  return {
    praxishub_base_url: val("praxishub_base_url") || cfg.praxishub_base_url,
    tenant_id: val("tenant_id"),
    api_key: val("api_key"),
    kim_host: val("kim_host"),
    kim_port: parseInt(val("kim_port") || "995", 10),
    kim_user: val("kim_user"),
    kim_password: val("kim_password"),
    kim_poll_seconds: 60,
    // Feld nur im Dashboard vorhanden → im Wizard bestehenden Wert bewahren.
    exchange_dir: hasEl("exchange_dir") ? val("exchange_dir") : cfg.exchange_dir,
    // Z1-DB-Felder gibt es nur im Dashboard → sonst bestehende Werte bewahren.
    z1_db_server: hasEl("z1_db_server") ? val("z1_db_server") : cfg.z1_db_server,
    z1_db_database: hasEl("z1_db_database") ? val("z1_db_database") || "Z1" : cfg.z1_db_database,
    z1_db_user: hasEl("z1_db_user") ? val("z1_db_user") : cfg.z1_db_user,
    z1_db_password: hasEl("z1_db_password") ? val("z1_db_password") : cfg.z1_db_password,
    z1_db_write_user: hasEl("z1_db_write_user") ? val("z1_db_write_user") : cfg.z1_db_write_user,
    z1_db_write_password: hasEl("z1_db_write_password") ? val("z1_db_write_password") : cfg.z1_db_write_password,
    z1_db_trust_cert: hasEl("z1_db_trust_cert") ? checked("z1_db_trust_cert") : cfg.z1_db_trust_cert,
    z1_hkp_lookback_months: hasEl("z1_hkp_lookback_months")
      ? parseInt(val("z1_hkp_lookback_months") || "24", 10)
      : cfg.z1_hkp_lookback_months,
    // Deutsches Komma erlaubt ("1,2"); leer/ungültig = 0 = keine Schätzung.
    z1_par_punktwert: hasEl("z1_par_punktwert")
      ? parseFloat(val("z1_par_punktwert").replace(",", ".")) || 0
      : cfg.z1_par_punktwert,
    writeback_contact: hasEl("writeback_contact") ? checked("writeback_contact") : cfg.writeback_contact,
    writeback_address: hasEl("writeback_address") ? checked("writeback_address") : cfg.writeback_address,
    writeback_cave: hasEl("writeback_cave") ? checked("writeback_cave") : cfg.writeback_cave,
    writeback_anamnese: hasEl("writeback_anamnese") ? checked("writeback_anamnese") : cfg.writeback_anamnese,
    // Notiz-Kanal hat keinen eigenen Schalter — er wird vom Modul „Rechnungen im
    // PVS ablegen" automatisch mit aktiviert (Karteikarten-Statusnotizen).
    writeback_notes: hasEl("pvs_file_invoices") ? checked("pvs_file_invoices") : cfg.writeback_notes,
    pvs_file_invoices: hasEl("pvs_file_invoices") ? checked("pvs_file_invoices") : cfg.pvs_file_invoices,
    writeback_new_patient: hasEl("writeback_new_patient") ? checked("writeback_new_patient") : cfg.writeback_new_patient,
    writeback_co_to_risk: hasEl("writeback_co_to_risk") ? checked("writeback_co_to_risk") : cfg.writeback_co_to_risk,
    // Das Rechnungs-Modul aktiviert die Archiv-Anzeige zwingend mit (sonst läge der
    // Beleg nur im PraxisArchiv, unsichtbar im Z1-Karteireiter „Archiv").
    writeback_archiv_link:
      hasEl("pvs_file_invoices") && checked("pvs_file_invoices")
        ? true
        : hasEl("writeback_archiv_link")
          ? checked("writeback_archiv_link")
          : cfg.writeback_archiv_link,
    z1_control_enabled: hasEl("z1_control_enabled") ? checked("z1_control_enabled") : cfg.z1_control_enabled,
    z1_control_hour: hasEl("z1_control_hour") ? clampInt(val("z1_control_hour"), 3, 0, 23) : cfg.z1_control_hour,
    z1_control_months: hasEl("z1_control_months") ? clampInt(val("z1_control_months"), 36, 1, 120) : cfg.z1_control_months,
    z1_control_column_map: hasEl("z1_control_column_map")
      ? parseColumnMap(val("z1_control_column_map"))
      : cfg.z1_control_column_map,
  };
}

function clampInt(raw: string, fallback: number, lo: number, hi: number): number {
  const n = parseInt(raw || String(fallback), 10);
  if (Number.isNaN(n)) return fallback;
  return Math.min(hi, Math.max(lo, n));
}

/// Textarea → Spalten-Override-Objekt. Leer = null (Auto-Erkennung). Ungültiges
/// JSON/kein Objekt wirft → saveFromDashboard fängt und meldet es (kein stiller Verlust).
function parseColumnMap(text: string): Record<string, string> | null {
  const t = text.trim();
  if (!t) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(t);
  } catch {
    throw new Error("Spalten-Zuordnung ist kein gültiges JSON.");
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new Error("Spalten-Zuordnung muss ein JSON-Objekt sein.");
  }
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(parsed as Record<string, unknown>)) {
    if (typeof v === "string" && v.trim()) out[k] = v.trim();
  }
  return Object.keys(out).length ? out : null;
}

function onApplyCode() {
  const decoded = decodeConnectionCode(val("code"));
  if (!decoded || !decoded.key) {
    setResult("cloud_result", "err", "Code ungültig oder unvollständig.");
    return;
  }
  ($("#praxishub_base_url") as HTMLInputElement).value = decoded.url ?? cfg.praxishub_base_url;
  ($("#tenant_id") as HTMLInputElement).value = decoded.tenant ?? "";
  ($("#api_key") as HTMLInputElement).value = decoded.key;
  ($(".collapsible") as HTMLDetailsElement).open = true;
  setResult("cloud_result", "ok", "Code übernommen – jetzt Verbindung testen.");
}

async function onTestCloud() {
  cfg = { ...cfg, ...collectFromWizard() };
  await call("save_config", { config: cfg });
  const res = await call<string>("test_cloud_connection");
  if (res) {
    setResult("cloud_result", "ok", `Verbunden: ${res}`);
    markStep("st1", true);
  } else {
    setResult("cloud_result", "err", "Keine Verbindung – URL/Key prüfen.");
  }
}

async function onTestKim() {
  cfg = { ...cfg, ...collectFromWizard() };
  await call("save_config", { config: cfg });
  const res = await call<string>("test_kim_connection");
  if (res) {
    setResult("kim_result", "ok", res);
    markStep("st2", true);
  } else {
    setResult("kim_result", "err", "KIM nicht erreichbar – Daten prüfen.");
  }
}

async function onFinish() {
  cfg = { ...cfg, ...collectFromWizard() };
  if (!isConfigured(cfg)) {
    toast("Bitte Praxishub- und KIM-Daten ausfüllen.");
    return;
  }
  await call("save_config", { config: cfg });
  await init(); // wechselt ins Dashboard
}

function markStep(id: string, done: boolean) {
  const el = $(`#${id}`);
  if (el) el.className = "step" + (done ? " done" : " active");
}
function setResult(id: string, kind: "ok" | "err", msg: string) {
  const el = $(`#${id}`);
  if (el) {
    el.className = "result " + kind;
    el.textContent = msg;
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Dashboard (eingerichteter Zustand)
// ─────────────────────────────────────────────────────────────────────────────

function renderDashboard() {
  $("#app").innerHTML = `
    <header><h1>Praxishub Connector</h1><span class="ver" id="ver"></span></header>
    <p class="sub">Lokale Brücke zwischen Ihrer Praxissoftware und Praxishub.</p>

    <div class="cards">
      ${statusCard("vdds", "PVS-Anbindung (VDDS-media)", "Dokumentenablage in die Patientenakte")}
      ${statusCard("kim", "HKP-Erkennung (KIM/EBZ)", "Genehmigte HKPs aus dem KIM-Postfach")}
      ${statusCard("cloud", "Praxishub-Cloud", "Verbindung zur Praxishub-Plattform")}
    </div>

    <section>
      <h2>Konfiguration</h2>
      <div class="row">
        <div class="field"><label>Praxishub-Tenant</label><input id="tenant_id" /></div>
        <div class="field"><label>API-Key</label><input id="api_key" type="password" /></div>
      </div>
      <div class="field"><label>Praxishub-URL</label><input id="praxishub_base_url" /></div>
      <div class="row">
        <div class="field"><label>KIM-Clientmodul Host</label><input id="kim_host" /></div>
        <div class="field"><label>KIM POP3-Port</label><input id="kim_port" /></div>
      </div>
      <div class="row">
        <div class="field"><label>KIM-Postfach (Benutzer)</label><input id="kim_user" /></div>
        <div class="field"><label>KIM-Passwort</label><input id="kim_password" type="password" /></div>
      </div>
      <div class="field"><label>VDDS-Austausch-Verzeichnis (leer = Temp)</label><input id="exchange_dir" placeholder="z. B. C:\\VDDS\\Austausch" /></div>
      <div class="actions">
        <button class="primary" id="save">Speichern</button>
        <button id="test_cloud">Cloud testen</button>
        <button id="test_kim">KIM testen</button>
      </div>
    </section>

    <section>
      <h2>PVS-Registrierung</h2>
      <p class="sub" style="margin-bottom:12px">Trägt Praxishub einmalig als VDDS-Modul in die Praxissoftware ein (einmalig Administrator-Rechte).</p>
      <div class="actions">
        <button id="register">Bei PVS registrieren</button>
        <button id="unregister">Registrierung entfernen</button>
      </div>
    </section>

    <section>
      <h2>Z1-Datenbank &amp; HKP-Tracking</h2>
      <p class="sub" style="margin-bottom:12px">Read-only-Zugriff auf die Z1-SQL-DB für HKP-Status und Stammdaten.</p>
      <div class="row">
        <div class="field"><label>SQL-Server / Instanz</label><input id="z1_db_server" placeholder="srv-fs\\z1" /></div>
        <div class="field"><label>Datenbank</label><input id="z1_db_database" placeholder="Z1" /></div>
      </div>
      <div class="row">
        <div class="field"><label>Read-only-Benutzer</label><input id="z1_db_user" placeholder="praxishub_ro" /></div>
        <div class="field"><label>Read-only-Passwort</label><input id="z1_db_password" type="password" /></div>
      </div>
      <label class="check" style="display:flex;align-items:center;gap:8px;margin:8px 0"><input type="checkbox" id="z1_db_trust_cert" /> Selbstsigniertes Serverzertifikat akzeptieren</label>
      <div class="field"><label>HKP-Verlauf: abgeschlossene Fälle bis (Monate zurück, 0 = unbegrenzt)</label><input id="z1_hkp_lookback_months" placeholder="24" /></div>
      <div class="field"><label>ePAR: KZV-Punktwert PAR in € (für Honorar-Schätzung; der ePAR-Antrag enthält keine Beträge — 0 = keine Schätzung)</label><input id="z1_par_punktwert" placeholder="z. B. 1,2846" /></div>
      <div class="actions"><button id="test_z1db">Z1-DB testen</button></div>
      <div class="result" id="z1_result"></div>

      <details class="collapsible" style="margin-top:14px">
        <summary>Read-only-Login anlegen (einmalig, mit Admin-Zugang)</summary>
        <p class="hint" style="margin-top:10px">Die Admin-Zugangsdaten werden <b>nicht gespeichert</b> — nur der erzeugte Read-only-Login (<code>praxishub_ro</code>) landet in der Konfiguration. Danach kannst du die Admin-Daten wieder verwerfen.</p>
        <div class="row">
          <div class="field"><label>Admin-Benutzer (z. B. sa)</label><input id="z1_admin_user" placeholder="sa" /></div>
          <div class="field"><label>Admin-Passwort</label><input id="z1_admin_password" type="password" /></div>
        </div>
        <div class="field"><label>Neues Read-only-Passwort</label><input id="z1_ro_password" type="password" /></div>
        <div class="actions"><button id="bootstrap_ro">Read-only-Login anlegen</button></div>
        <div class="result" id="z1_bootstrap_result"></div>
      </details>

      <h3>Daten aus der Online-Aufnahme übernehmen</h3>
      <p class="sub" style="margin-bottom:10px">Was Patienten vor dem Termin online ausfüllen, in die Patientenakte übernehmen. Braucht einen schreibfähigen Login; jede Funktion ist einzeln aktivierbar.</p>
      <div class="row">
        <div class="field"><label>Schreib-Benutzer</label><input id="z1_db_write_user" placeholder="Schreib-Login" /></div>
        <div class="field"><label>Schreib-Passwort</label><input id="z1_db_write_password" type="password" /></div>
      </div>
      <label class="check"><input type="checkbox" id="writeback_contact" /> Kontaktdaten (Telefon, E-Mail)</label>
      <label class="check"><input type="checkbox" id="writeback_address" /> Anschrift <span class="hint">(überschreibt die vorhandene Adresse)</span></label>
      <label class="check"><input type="checkbox" id="writeback_cave" /> Allergien &amp; Warnhinweise <span class="hint">(als Risikoanamnese)</span></label>
      <label class="check"><input type="checkbox" id="writeback_anamnese" /> Krankengeschichte / Anamnese</label>
      <label class="check"><input type="checkbox" id="writeback_archiv_link" /> Abgelegte Dokumente im Karteireiter „Archiv" anzeigen</label>

      <h3>Rechnungen im PVS ablegen</h3>
      <p class="sub" style="margin-bottom:10px">Rechnungen und Stornos aus dem Praxishub-Rechnungsmodul ins PVS-Archiv legen und den Zahlungsstatus in der Karteikarte vermerken. Braucht den schreibfähigen Login. Aktiviert die Anzeige im Karteireiter „Archiv" und die Statusnotiz automatisch mit.</p>
      <label class="check"><input type="checkbox" id="pvs_file_invoices" /> Rechnungen &amp; Stornos im PVS ablegen <span class="hint">(vermerkt „bezahlt/offen" automatisch in der Karteikarte)</span></label>

      <details class="collapsible" style="margin-top:14px">
        <summary>Erweiterte Rückschreib-Optionen</summary>
        <label class="check" style="margin-top:10px"><input type="checkbox" id="writeback_co_to_risk" /> Abweichende Anschrift (c/o) als Hinweis vermerken</label>
        <label class="check"><input type="checkbox" id="writeback_new_patient" /> Neupatient anlegen <span class="hint">(Vorsicht: Dubletten-Risiko beim Kartenstecken)</span></label>
      </details>

      <h3 style="margin-top:20px">Praxis-Steuerung (Umsatz- &amp; Leistungs-Sync)</h3>
      <p class="sub" style="margin-bottom:10px">Liest einmal täglich aggregierte Umsatz-/Abrechnungsdaten (read-only) und speist das Modul „Praxis-Steuerung" in Praxishub. Nur Aggregate, keine Klartext-Patientendaten. War der PC nachts aus, wird der Lauf morgens nachgeholt.</p>
      <label class="check" style="display:flex;align-items:center;gap:8px;margin:6px 0"><input type="checkbox" id="z1_control_enabled" /> Umsatz-/Leistungs-Sync aktivieren</label>
      <div class="row">
        <div class="field"><label>Früheste Stunde (0–23, Standard 3)</label><input id="z1_control_hour" placeholder="3" /></div>
        <div class="field"><label>Monats-Aggregate: Zeitfenster (Monate)</label><input id="z1_control_months" placeholder="36" /></div>
      </div>
      <details class="collapsible" style="margin-top:8px">
        <summary>Spalten-Zuordnung (nur falls die automatische Erkennung Felder offen lässt)</summary>
        <p class="hint" style="margin-top:10px">JSON-Objekt: logischer Schlüssel → echter Z1-Spaltenname. Leer lassen für automatische Erkennung. Beispiel: <code>{"beh_datum":"LEISTDATUM","beh_art":"ABRECHNUNGSART"}</code></p>
        <textarea id="z1_control_column_map" rows="5" style="width:100%;font-family:monospace;font-size:12px" placeholder='{ "beh_datum": "...", "beh_art": "..." }'></textarea>
      </details>

      <div class="actions"><button class="primary" id="save2">Speichern</button></div>
    </section>

    <section><h2>Verlauf</h2><div class="log" id="log">—</div></section>
    <div class="toast" id="toast"></div>
  `;

  applyConfig(cfg);
  syncInvoiceModule();
  $("#pvs_file_invoices")?.addEventListener("change", syncInvoiceModule);
  $("#save").addEventListener("click", saveFromDashboard);
  $("#test_cloud").addEventListener("click", () => testConn("test_cloud_connection", "Cloud"));
  $("#test_kim").addEventListener("click", () => testConn("test_kim_connection", "KIM"));
  $("#register").addEventListener("click", () => action("register_with_pvs", "Registrierung gestartet …"));
  $("#unregister").addEventListener("click", () => action("unregister_from_pvs", "Registrierung entfernt."));
  $("#test_z1db").addEventListener("click", onTestZ1);
  $("#bootstrap_ro").addEventListener("click", onBootstrapRo);
  $("#save2").addEventListener("click", saveFromDashboard);
}

/** Modul „Rechnungen im PVS ablegen" zieht die Archiv-Anzeige zwingend mit:
 *  ist es an, wird „Dokumente im Archiv anzeigen" angehakt und gesperrt. */
function syncInvoiceModule() {
  const inv = $("#pvs_file_invoices") as HTMLInputElement | null;
  const arch = $("#writeback_archiv_link") as HTMLInputElement | null;
  if (!inv || !arch) return;
  if (inv.checked) {
    // Vorherige Nutzer-Wahl merken, dann erzwingen + sperren.
    if (arch.dataset.prev === undefined) arch.dataset.prev = arch.checked ? "1" : "0";
    arch.checked = true;
    arch.disabled = true;
    arch.title = "Durch „Rechnungen im PVS ablegen" automatisch aktiviert";
  } else {
    // Modul aus → gemerkten Zustand wiederherstellen (nicht fälschlich angehakt lassen).
    if (arch.dataset.prev !== undefined) {
      arch.checked = arch.dataset.prev === "1";
      delete arch.dataset.prev;
    }
    arch.disabled = false;
    arch.title = "";
  }
}

function statusCard(id: string, title: string, desc: string) {
  return `<div class="card">
    <span class="dot" id="dot_${id}"></span>
    <div class="body"><div class="title">${title}</div><div class="desc" id="desc_${id}">${desc}</div></div>
  </div>`;
}

function applyStatus(s: StatusSnapshot) {
  const v = $("#ver");
  if (v) v.textContent = "v" + s.version;
  for (const k of ["vdds", "kim", "cloud"] as const) {
    const dot = $(`#dot_${k}`);
    if (dot) dot.className = "dot " + (s[k].state === "unknown" ? "" : s[k].state);
    if (s[k].detail) {
      const d = $(`#desc_${k}`);
      if (d) d.textContent = s[k].detail;
    }
  }
  if (s.last_hkp_at) {
    const l = $("#log");
    if (l) l.textContent = `Letzter HKP erkannt: ${s.last_hkp_at}`;
  }
}

function applyConfig(c: ConnectorConfig) {
  setIf("praxishub_base_url", c.praxishub_base_url);
  setIf("tenant_id", c.tenant_id);
  setIf("api_key", c.api_key);
  setIf("kim_host", c.kim_host);
  setIf("kim_port", String(c.kim_port ?? 995));
  setIf("kim_user", c.kim_user);
  setIf("kim_password", c.kim_password);
  setIf("exchange_dir", c.exchange_dir);
  setIf("z1_db_server", c.z1_db_server);
  setIf("z1_db_database", c.z1_db_database || "Z1");
  setIf("z1_db_user", c.z1_db_user);
  setIf("z1_db_password", c.z1_db_password);
  setIf("z1_db_write_user", c.z1_db_write_user);
  setIf("z1_db_write_password", c.z1_db_write_password);
  setIf("z1_hkp_lookback_months", String(c.z1_hkp_lookback_months ?? 24));
  // setIf setzt nur nicht-leere Werte — 0 (= aus) bleibt als Placeholder leer.
  setIf("z1_par_punktwert", c.z1_par_punktwert ? String(c.z1_par_punktwert).replace(".", ",") : "");
  setChecked("z1_db_trust_cert", c.z1_db_trust_cert ?? true);
  setChecked("writeback_contact", c.writeback_contact);
  setChecked("writeback_address", c.writeback_address);
  setChecked("writeback_cave", c.writeback_cave);
  setChecked("writeback_anamnese", c.writeback_anamnese);
  setChecked("writeback_new_patient", c.writeback_new_patient);
  setChecked("writeback_co_to_risk", c.writeback_co_to_risk);
  setChecked("writeback_archiv_link", c.writeback_archiv_link ?? false);
  setChecked("pvs_file_invoices", c.pvs_file_invoices ?? false);
  setChecked("z1_control_enabled", c.z1_control_enabled ?? false);
  setIf("z1_control_hour", String(c.z1_control_hour ?? 3));
  setIf("z1_control_months", String(c.z1_control_months ?? 36));
  setIf("z1_control_column_map", c.z1_control_column_map ? JSON.stringify(c.z1_control_column_map, null, 2) : "");
}

async function onTestZ1() {
  cfg = collectFromWizard();
  await call("save_config", { config: cfg });
  const res = await call<string>("test_z1db_connection");
  setResult("z1_result", res ? "ok" : "err", res ? res : "Keine Verbindung – Server/Login prüfen.");
}

async function onBootstrapRo() {
  const server = val("z1_db_server");
  const adminUser = val("z1_admin_user");
  const adminPassword = val("z1_admin_password");
  const roPassword = val("z1_ro_password");
  if (!server || !adminUser || !adminPassword || !roPassword) {
    setResult("z1_bootstrap_result", "err", "Server, Admin-Zugang und neues Read-only-Passwort ausfüllen.");
    return;
  }
  const res = await call<string>("bootstrap_z1_readonly", {
    server,
    adminUser,
    adminPassword,
    roPassword,
    trustCert: checked("z1_db_trust_cert"),
  });
  if (res) {
    setResult("z1_bootstrap_result", "ok", res);
    const loaded = await call<ConnectorConfig>("get_config"); // RO-Login wurde serverseitig gespeichert
    if (loaded) {
      cfg = loaded;
      applyConfig(cfg);
    }
  } else {
    setResult("z1_bootstrap_result", "err", "Anlegen fehlgeschlagen – Admin-Zugang prüfen.");
  }
}

async function saveFromDashboard() {
  try {
    cfg = collectFromWizard();
  } catch (e) {
    toast(`Nicht gespeichert: ${(e as Error).message}`);
    return;
  }
  const ok = await call("save_config", { config: cfg });
  if (ok !== null) {
    toast("Gespeichert.");
    refresh();
  }
}

async function testConn(cmd: string, label: string) {
  const res = await call<string>(cmd);
  toast(res ? `${label}: ${res}` : `${label}: fehlgeschlagen`);
  refresh();
}

async function action(cmd: string, msg: string) {
  const res = await call<string>(cmd);
  if (res !== null) {
    toast(msg);
    refresh();
  }
}

async function refresh() {
  const s = await call<StatusSnapshot>("get_status");
  if (s) applyStatus(s);
}

// ─────────────────────────────────────────────────────────────────────────────

let pollTimer: number | undefined;

async function init() {
  const loaded = await call<ConnectorConfig>("get_config");
  if (loaded) cfg = loaded;

  if (isConfigured(cfg)) {
    renderDashboard();
    await refresh();
    if (pollTimer === undefined) pollTimer = window.setInterval(refresh, 5000);
  } else {
    if (pollTimer !== undefined) {
      clearInterval(pollTimer);
      pollTimer = undefined;
    }
    renderWizard();
  }
}

init();
