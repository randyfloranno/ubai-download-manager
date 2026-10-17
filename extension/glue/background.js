// UDM extension glue: loads the Rust/wasm core and forwards WebExtension
// events to it. All decisions live in Rust (crates/udm-ext); keep this tiny.
import init, * as udm from "./udm_ext.js";

const api = globalThis.browser ?? globalThis.chrome;
const ready = init();
const lang = () => api.i18n.getUILanguage();

async function settingsJson() {
  const got = await api.storage.local.get("settings");
  return got.settings ? JSON.stringify(got.settings) : udm.default_settings();
}

async function cookiesFor(url) {
  try {
    return udm.cookie_header(JSON.stringify(await api.cookies.getAll({ url })));
  } catch (_) {
    return "";
  }
}

async function nativeSend(message) {
  try {
    return await api.runtime.sendNativeMessage(udm.host_name(), message);
  } catch (_) {
    return null;
  }
}

// Returns true when UDM took the download over.
async function forward(event) {
  await ready;
  const decision = JSON.parse(udm.decide(await settingsJson(), JSON.stringify(event)));
  if (decision.action !== "capture") return false;
  const message = decision.message;
  const cookie = await cookiesFor(decision.cookie_url);
  if (cookie) message.cookie = cookie;
  const reply = await nativeSend(message);
  const shown = JSON.parse(udm.reply(lang(), JSON.stringify(reply ?? null)));
  await api.action.setBadgeText({ text: shown.badge });
  await api.action.setTitle({ title: shown.text });
  return shown.captured;
}

async function createMenus() {
  await ready;
  await api.contextMenus.removeAll();
  for (const item of JSON.parse(udm.menus(lang()))) api.contextMenus.create(item);
}

api.runtime.onInstalled.addListener(() => createMenus());
api.runtime.onStartup.addListener(() => createMenus());

api.contextMenus.onClicked.addListener((info) => {
  forward({ kind: "menu", info });
});

api.downloads.onCreated.addListener(async (item) => {
  if (await forward({ kind: "download", item })) {
    try {
      await api.downloads.cancel(item.id);
      await api.downloads.erase({ id: item.id });
    } catch (_) {
      // The browser may already have finished a tiny download.
    }
  }
});
