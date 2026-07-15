"use strict";

const api = globalThis.browser ?? globalThis.chrome;
const promiseResponses = typeof globalThis.browser !== "undefined";
const core = globalThis.ProseContentCore;
const state = new core.EditorState();

function currentSurface() {
  return core.detectSurface(location);
}

function selectionContext() {
  const surface = currentSurface();
  if (!surface) return {selection: "", surrounding_context: "", surface: null};
  const active = document.activeElement;
  if (active && (active.tagName === "TEXTAREA" || active.tagName === "INPUT")) {
    if (core.assistEditor(location, active) !== active) return {selection: "", surrounding_context: "", surface};
    const start = active.selectionStart ?? 0;
    const end = active.selectionEnd ?? start;
    return {...state.selectInput(active, start, end), surface};
  }
  const selected = getSelection();
  if (!selected || selected.rangeCount === 0) return {selection: "", surrounding_context: "", surface};
  const range = selected.getRangeAt(0);
  const node = range.commonAncestorContainer.nodeType === Node.ELEMENT_NODE
    ? range.commonAncestorContainer
    : range.commonAncestorContainer.parentElement;
  const candidate = node?.closest?.('[contenteditable="true"]');
  const editor = core.assistEditor(location, candidate);
  if (!editor) return {selection: "", surrounding_context: "", surface};
  return {...state.selectRange(editor, range, selected.toString(), core.editorText(editor)), surface};
}

api.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message.type === "prose-selection") {
    return core.respondAsync(promiseResponses, sendResponse, selectionContext);
  }
  if (message.type === "prose-replace") state.replace(message.candidate, message.draft);
  if (message.type === "prose-error") console.warn(`prose: ${message.message}`);
  return undefined;
});

function sendCapture(editor, source, action) {
  const surface = currentSurface();
  if (!surface) return;
  const capture = state.capture(editor, {
    id: crypto.randomUUID(),
    surface,
    url: `${location.origin}${location.pathname}`,
    source,
  }, action);
  if (capture) core.sendOneWay(api, promiseResponses, {type: "prose-capture", capture});
}

document.addEventListener("submit", (event) => {
  const editor = core.nativeEditor(location, event.target);
  if (editor) sendCapture(editor, "browser-native-submit", event);
}, true);

document.addEventListener("click", (event) => {
  const editor = core.linearClickEditor(location, event.target);
  if (editor) sendCapture(editor, "browser-linear-spa-click", event);
}, true);

// Any user-originated edit permanently breaks the exact candidate-to-draft
// association, even if the text is later changed back to the candidate.
// Replacement-generated input fires before EditorState records its new
// association, so it does not invalidate the replacement itself.
document.addEventListener("input", (event) => {
  const target = event.target;
  const editor = target?.closest?.('[contenteditable="true"]') ?? target;
  state.invalidate(editor);
}, true);

const observer = new MutationObserver(() => state.clearDisconnected());
observer.observe(document, {childList: true, subtree: true});

function navigation() {
  state.clearNavigation();
}
for (const method of ["pushState", "replaceState"]) {
  const original = history[method];
  history[method] = function (...args) {
    const result = original.apply(this, args);
    navigation();
    return result;
  };
}
addEventListener("popstate", navigation);
addEventListener("hashchange", navigation);
