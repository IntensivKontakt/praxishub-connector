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
  };
}

const $ = <T extends HTMLElement = HTMLElement>(sel: string) =>
  document.querySelector(sel) as T;
const val = (id: string) => ($(`#${id}`) as HTMLInputElement)?.value.trim() ?? "";

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
    exchange_dir: ($("#exchange_dir") as HTMLInputElement | null) ? val("exchange_dir") : cfg.exchange_dir,
  };
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

    <section><h2>Verlauf</h2><div class="log" id="log">—</div></section>
    <div class="toast" id="toast"></div>
  `;

  applyConfig(cfg);
  $("#save").addEventListener("click", saveFromDashboard);
  $("#test_cloud").addEventListener("click", () => testConn("test_cloud_connection", "Cloud"));
  $("#test_kim").addEventListener("click", () => testConn("test_kim_connection", "KIM"));
  $("#register").addEventListener("click", () => action("register_with_pvs", "Registrierung gestartet …"));
  $("#unregister").addEventListener("click", () => action("unregister_from_pvs", "Registrierung entfernt."));
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
}

async function saveFromDashboard() {
  cfg = collectFromWizard();
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
