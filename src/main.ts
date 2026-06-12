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
}

const $ = <T extends HTMLElement = HTMLElement>(sel: string) =>
  document.querySelector(sel) as T;

function toast(msg: string) {
  const t = $("#toast");
  t.textContent = msg;
  t.classList.add("show");
  setTimeout(() => t.classList.remove("show"), 2600);
}

// invoke, das auch ohne Tauri-Backend (z. B. reiner Vite-Dev) nicht crasht.
async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T | null> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    console.warn(`invoke ${cmd} failed:`, e);
    toast(String(e));
    return null;
  }
}

function render() {
  $("#app").innerHTML = `
    <header>
      <h1>Praxishub Connector</h1>
      <span class="ver" id="ver"></span>
    </header>
    <p class="sub">Lokale Brücke zwischen Ihrer Praxissoftware und Praxishub.</p>

    <div class="cards">
      ${statusCard("vdds", "PVS-Anbindung (VDDS-media)", "Dokumentenablage in die Patientenakte")}
      ${statusCard("kim", "HKP-Erkennung (KIM/EBZ)", "Genehmigte HKPs aus dem KIM-Postfach")}
      ${statusCard("cloud", "Praxishub-Cloud", "Verbindung zur Praxishub-Plattform")}
    </div>

    <section>
      <h2>Konfiguration</h2>
      <div class="row">
        <div class="field"><label>Praxishub-Tenant</label><input id="tenant_id" placeholder="praxis-xyz" /></div>
        <div class="field"><label>API-Key</label><input id="api_key" type="password" placeholder="•••••••••" /></div>
      </div>
      <div class="field"><label>Praxishub-URL</label><input id="praxishub_base_url" placeholder="https://api.praxishub.ai" /></div>
      <div class="row">
        <div class="field"><label>KIM-Clientmodul Host</label><input id="kim_host" placeholder="127.0.0.1" /></div>
        <div class="field"><label>KIM POP3-Port</label><input id="kim_port" placeholder="995" /></div>
      </div>
      <div class="row">
        <div class="field"><label>KIM-Postfach (Benutzer)</label><input id="kim_user" placeholder="praxis@kim.telematik" /></div>
        <div class="field"><label>KIM-Passwort</label><input id="kim_password" type="password" placeholder="•••••••••" /></div>
      </div>
      <div class="actions">
        <button class="primary" id="save">Speichern</button>
        <button id="test_cloud">Cloud testen</button>
        <button id="test_kim">KIM testen</button>
      </div>
    </section>

    <section>
      <h2>PVS-Registrierung</h2>
      <p class="sub" style="margin-bottom:12px">
        Trägt Praxishub einmalig als VDDS-Modul in die Praxissoftware ein. Erfordert
        einmalig Administrator-Rechte (Windows-Abfrage).
      </p>
      <div class="actions">
        <button id="register">Bei PVS registrieren</button>
        <button id="unregister">Registrierung entfernen</button>
      </div>
    </section>

    <section>
      <h2>Verlauf</h2>
      <div class="log" id="log">—</div>
    </section>

    <div class="toast" id="toast"></div>
  `;

  $("#save").addEventListener("click", saveConfig);
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
  $("#ver").textContent = "v" + s.version;
  for (const k of ["vdds", "kim", "cloud"] as const) {
    const dot = $(`#dot_${k}`);
    dot.className = "dot " + (s[k].state === "unknown" ? "" : s[k].state);
    if (s[k].detail) $(`#desc_${k}`).textContent = s[k].detail;
  }
  if (s.last_hkp_at) $("#log").textContent = `Letzter HKP erkannt: ${s.last_hkp_at}`;
}

function applyConfig(c: ConnectorConfig) {
  ($("#praxishub_base_url") as HTMLInputElement).value = c.praxishub_base_url ?? "";
  ($("#tenant_id") as HTMLInputElement).value = c.tenant_id ?? "";
  ($("#api_key") as HTMLInputElement).value = c.api_key ?? "";
  ($("#kim_host") as HTMLInputElement).value = c.kim_host ?? "";
  ($("#kim_port") as HTMLInputElement).value = String(c.kim_port ?? 995);
  ($("#kim_user") as HTMLInputElement).value = c.kim_user ?? "";
  ($("#kim_password") as HTMLInputElement).value = c.kim_password ?? "";
}

function readConfig(): ConnectorConfig {
  const v = (id: string) => ($(`#${id}`) as HTMLInputElement).value.trim();
  return {
    praxishub_base_url: v("praxishub_base_url"),
    tenant_id: v("tenant_id"),
    api_key: v("api_key"),
    kim_host: v("kim_host"),
    kim_port: parseInt(v("kim_port") || "995", 10),
    kim_user: v("kim_user"),
    kim_password: v("kim_password"),
    kim_poll_seconds: 60,
  };
}

async function saveConfig() {
  const ok = await call("save_config", { config: readConfig() });
  if (ok !== null) { toast("Gespeichert."); refresh(); }
}

async function testConn(cmd: string, label: string) {
  const res = await call<string>(cmd);
  if (res !== null) toast(`${label}: ${res}`);
  refresh();
}

async function action(cmd: string, msg: string) {
  const res = await call<string>(cmd);
  if (res !== null) { toast(msg); refresh(); }
}

async function refresh() {
  const s = await call<StatusSnapshot>("get_status");
  if (s) applyStatus(s);
}

async function init() {
  render();
  const c = await call<ConnectorConfig>("get_config");
  if (c) applyConfig(c);
  await refresh();
  setInterval(refresh, 5000);
}

init();
