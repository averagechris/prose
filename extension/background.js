const api = globalThis.browser ?? globalThis.chrome;
const endpoint = "http://127.0.0.1:37673/v1";
const menuPrefix = "prose-verb:";
const captureQueueKey = "proseCaptureQueue";
let captureWork = Promise.resolve();

async function request(path, body) {
  const response = await fetch(`${endpoint}/${path}`, {
    method: body ? "POST" : "GET",
    headers: body ? {"Content-Type": "application/json"} : {},
    body: body ? JSON.stringify(body) : undefined,
  });
  const value = await response.json();
  if (!response.ok) throw new Error(value.error?.message ?? `prose returned ${response.status}`);
  return value;
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

async function enqueueCapture(capture) {
  const stored = await api.storage.local.get(captureQueueKey);
  const queue = Array.isArray(stored[captureQueueKey]) ? stored[captureQueueKey] : [];
  queue.push(capture);
  await api.storage.local.set({[captureQueueKey]: queue});
}

async function flushCaptures() {
  const stored = await api.storage.local.get(captureQueueKey);
  const queue = Array.isArray(stored[captureQueueKey]) ? stored[captureQueueKey] : [];
  let sent = 0;
  try {
    for (const capture of queue) {
      await request("capture", {action: "record", capture});
      sent += 1;
    }
  } finally {
    if (sent > 0) await api.storage.local.set({[captureQueueKey]: queue.slice(sent)});
  }
}

api.runtime.onInstalled.addListener(refreshMenus);
api.runtime.onStartup.addListener(() => {
  refreshMenus();
  flushCaptures().catch(() => undefined);
});
api.alarms.create("refresh-prose-verbs", {periodInMinutes: 5});
api.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === "refresh-prose-verbs") refreshMenus();
  flushCaptures().catch(() => undefined);
});

api.contextMenus.onClicked.addListener(async (info, tab) => {
  if (!String(info.menuItemId).startsWith(menuPrefix) || !tab?.id) return;
  const verb = String(info.menuItemId).slice(menuPrefix.length);
  try {
    const page = await api.tabs.sendMessage(tab.id, {type: "prose-selection"});
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

api.runtime.onMessage.addListener((message) => {
  if (message.type !== "prose-capture") return undefined;
  captureWork = captureWork.then(() => enqueueCapture(message.capture)
    .then(flushCaptures)
    .catch(() => undefined));
  return captureWork;
});
