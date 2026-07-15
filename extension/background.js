"use strict";

if (!globalThis.ProseQueue && typeof importScripts === "function") importScripts("queue.js");
if (!globalThis.ProseContentCore && typeof importScripts === "function") importScripts("content-core.js");

const api = globalThis.browser ?? globalThis.chrome;
const promiseResponses = typeof globalThis.browser !== "undefined";
const endpoint = "http://127.0.0.1:37673/v1";
const menuPrefix = "prose-verb:";
const requestTimeoutMs = 10_000;

class HttpError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

async function request(path, body) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), requestTimeoutMs);
  try {
    const response = await fetch(`${endpoint}/${path}`, {
      method: body ? "POST" : "GET",
      headers: body ? {"Content-Type": "application/json"} : {},
      body: body ? JSON.stringify(body) : undefined,
      signal: controller.signal,
    });
    let value = {};
    try {
      value = await response.json();
    } catch (_) {
      // Status is sufficient for retry classification; never retain response bodies.
    }
    if (!response.ok) throw new HttpError(response.status, value.error?.message ?? `prose returned ${response.status}`);
    return value;
  } finally {
    clearTimeout(timeout);
  }
}

async function refreshMenus() {
  await api.contextMenus.removeAll();
  try {
    const result = await request("verb", {action: "list", context: null, pack: null});
    api.contextMenus.create({id: "prose-root", title: "Edit with prose", contexts: ["selection"]});
    for (const item of result.items) {
      api.contextMenus.create({
        id: `${menuPrefix}${item.verb.name}`,
        parentId: "prose-root",
        title: item.verb.description || item.verb.name,
        contexts: ["selection"],
      });
    }
  } catch (_) {
    api.contextMenus.create({id: "prose-unavailable", title: "prose unavailable", contexts: ["selection"], enabled: false});
  }
}

const captures = globalThis.ProseQueue.createCaptureManager({
  storage: api.storage.local,
  send: (capture) => request("capture", {action: "record", capture}),
});

api.runtime.onInstalled.addListener(() => {
  refreshMenus();
  captures.flush().catch(() => undefined);
});
api.runtime.onStartup.addListener(() => {
  refreshMenus();
  captures.flush().catch(() => undefined);
});
api.alarms.create("refresh-prose-verbs", {periodInMinutes: 5});
api.alarms.create("retry-prose-captures", {periodInMinutes: 1});
api.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === "refresh-prose-verbs") refreshMenus();
  if (alarm.name === "retry-prose-captures") captures.flush().catch(() => undefined);
});

api.contextMenus.onClicked.addListener(async (info, tab) => {
  if (!String(info.menuItemId).startsWith(menuPrefix) || !tab?.id) return;
  const verb = String(info.menuItemId).slice(menuPrefix.length);
  try {
    const page = await api.tabs.sendMessage(tab.id, {type: "prose-selection"});
    if (!page.surface) throw new Error("unsupported page");
    const mapping = await request("surface", {name: page.surface, pack: null});
    const result = await request("assist", {
      surface: page.surface,
      context: mapping.context.name,
      verb,
      pack: mapping.pack_id,
      selection: page.selection,
      surrounding_context: page.surrounding_context,
      draft: null,
    });
    await api.tabs.sendMessage(tab.id, {type: "prose-replace", candidate: result.candidate, draft: result.draft});
  } catch (error) {
    await api.tabs.sendMessage(tab.id, {type: "prose-error", message: String(error.message ?? error)});
  }
});

api.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message.type !== "prose-capture") return undefined;
  return globalThis.ProseContentCore.respondAsync(
    promiseResponses,
    sendResponse,
    () => captures.capture(message.capture).then(() => ({ok: true})),
  );
});
