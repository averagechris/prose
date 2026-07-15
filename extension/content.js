const api = globalThis.browser ?? globalThis.chrome;
let pendingSelection = null;
let lastDraft = null;

function surface() {
  if (location.hostname === "github.com") return "github-pr";
  if (location.hostname === "linear.app") return "linear";
  return location.hostname;
}

function selectionContext() {
  const active = document.activeElement;
  if (active && (active.tagName === "TEXTAREA" || active.tagName === "INPUT")) {
    const start = active.selectionStart ?? 0;
    const end = active.selectionEnd ?? start;
    pendingSelection = {kind: "input", element: active, start, end};
    return {selection: active.value.slice(start, end), surrounding_context: active.value};
  }
  const selected = getSelection();
  if (!selected || selected.rangeCount === 0) return {selection: "", surrounding_context: ""};
  const range = selected.getRangeAt(0).cloneRange();
  pendingSelection = {kind: "range", range};
  const container = range.commonAncestorContainer.parentElement ?? document.body;
  return {selection: selected.toString(), surrounding_context: container.innerText?.slice(0, 8000) ?? ""};
}

function replaceSelection(candidate) {
  if (!pendingSelection) return;
  if (pendingSelection.kind === "input" && pendingSelection.element.isConnected) {
    const {element, start, end} = pendingSelection;
    element.setRangeText(candidate, start, end, "end");
    element.dispatchEvent(new InputEvent("input", {bubbles: true, inputType: "insertText", data: candidate}));
  } else if (pendingSelection.kind === "range") {
    const range = pendingSelection.range;
    range.deleteContents();
    range.insertNode(document.createTextNode(candidate));
    range.commonAncestorContainer.parentElement?.dispatchEvent(new InputEvent("input", {bubbles: true, inputType: "insertText", data: candidate}));
  }
  pendingSelection = null;
}

api.runtime.onMessage.addListener((message) => {
  if (message.type === "prose-selection") return Promise.resolve({...selectionContext(), surface: surface()});
  if (message.type === "prose-replace") {
    replaceSelection(message.candidate);
    lastDraft = message.draft;
  }
  if (message.type === "prose-error") console.warn(`prose: ${message.message}`);
  return undefined;
});

function editableText(target) {
  const form = target.closest?.("form");
  const root = form ?? target.closest?.("[role=dialog]") ?? document;
  const editable = root.querySelector?.("textarea, [contenteditable=true]");
  if (!editable) return "";
  return "value" in editable ? editable.value : editable.innerText;
}

function capture(target) {
  const content = editableText(target).trim();
  if (!content) return;
  api.runtime.sendMessage({type: "prose-capture", capture: {
    id: crypto.randomUUID(),
    surface: surface(),
    url: `${location.origin}${location.pathname}`,
    content,
    draft: lastDraft,
    metadata: {source: "browser-submit-event"},
  }}).catch(() => undefined);
  lastDraft = null;
}

document.addEventListener("submit", (event) => capture(event.target), true);
document.addEventListener("click", (event) => {
  const target = event.target.closest?.("button, [role=button]");
  if (!target) return;
  const label = `${target.textContent ?? ""} ${target.getAttribute("aria-label") ?? ""}`.toLowerCase();
  if (/submit|comment|review|reply|create issue|save/.test(label)) capture(target);
}, true);
