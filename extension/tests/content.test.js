const assert = require("node:assert/strict");
const {test} = require("node:test");
const {
  EditorState,
  assistEditor,
  boundedContext,
  detectSurface,
  linearClickEditor,
  nativeEditor,
  respondAsync,
  sendOneWay,
} = require("../content-core.js");

global.InputEvent = class InputEvent {
  constructor(type, options) { this.type = type; Object.assign(this, options); }
};

class FrameworkInput {
  constructor(value) {
    this._value = value;
    this.isConnected = true;
    this.events = [];
  }
  get value() { return this._value; }
  set value(value) { this._value = value; this.setterCalls = (this.setterCalls ?? 0) + 1; }
  setSelectionRange(start, end) { this.selection = [start, end]; }
  dispatchEvent(event) { this.events.push(event); return true; }
}

function details(id = "stable-id") {
  return {id, surface: "github-pr", url: "https://github.com/o/r/pull/7", source: "test"};
}

test("textarea replacement uses the native setter and an input event", () => {
  const editor = new FrameworkInput("before AFTER");
  const state = new EditorState();
  state.selectInput(editor, 7, 12);
  assert.equal(state.replace("after", "draft-1"), true);
  assert.equal(editor.value, "before after");
  assert.equal(editor.setterCalls, 1);
  assert.deepEqual(editor.selection, [12, 12]);
  assert.deepEqual(editor.events.map((event) => event.type), ["beforeinput", "input"]);
});

test("contenteditable replacement dispatches input and records state only on success", () => {
  const editor = {
    isConnected: true,
    innerText: "old",
    events: [],
    ownerDocument: {createTextNode: (text) => ({text})},
    dispatchEvent(event) { this.events.push(event); },
  };
  const range = {
    cloneRange() { return this; },
    deleteContents() { editor.innerText = ""; },
    insertNode(node) { editor.innerText = node.text; },
    collapse() {},
  };
  const state = new EditorState();
  state.selectRange(editor, range, "old", "old");
  assert.equal(state.replace("new", "draft-2"), true);
  assert.equal(editor.innerText, "new");
  assert.deepEqual(editor.events.map((event) => event.type), ["beforeinput", "input"]);
  assert.equal(state.capture(editor, details()).draft, "draft-2");

  state.selectRange(editor, {...range, cloneRange() { return this; }, insertNode() { throw new Error("no"); }}, "new", "new");
  assert.equal(state.replace("bad", "draft-bad"), false);
  editor.innerText = "bad";
  assert.equal(state.capture(editor, details("next")).draft, null);
});

test("a canceled framework beforeinput does not replace or set draft state", () => {
  const editor = new FrameworkInput("old");
  editor.dispatchEvent = (event) => event.type !== "beforeinput";
  const state = new EditorState();
  state.selectInput(editor, 0, 3);
  assert.equal(state.replace("new", "draft"), false);
  assert.equal(editor.value, "old");
  assert.equal(state.capture(editor, details()).draft, null);
});

test("draft association is exact to editor and candidate and preserves whitespace", () => {
  const first = new FrameworkInput("candidate");
  const other = new FrameworkInput("candidate");
  const state = new EditorState();
  state.selectInput(first, 0, first.value.length);
  state.replace("candidate", "draft-a");
  assert.equal(state.capture(first, details()).draft, "draft-a");
  assert.equal(state.capture(other, details("other")).draft, null);

  first.value = " candidate ";
  const edited = state.capture(first, details("edited"));
  assert.equal(edited.content, " candidate ");
  assert.equal(edited.draft, null);
});

test("any human edit permanently invalidates a draft association", () => {
  const editor = new FrameworkInput("candidate");
  const state = new EditorState();
  state.selectInput(editor, 0, editor.value.length);
  state.replace("candidate", "draft-a");
  editor.value = "human edit";
  state.invalidate(editor);
  editor.value = "candidate";
  assert.equal(state.capture(editor, details("restored")).draft, null);
});

test("removed editors and SPA navigation clear candidate state", () => {
  const editor = new FrameworkInput("x");
  const state = new EditorState();
  state.selectInput(editor, 0, 1);
  state.replace("x", "draft");
  editor.isConnected = false;
  state.clearDisconnected();
  editor.isConnected = true;
  assert.equal(state.capture(editor, details()).draft, null);

  state.selectInput(editor, 0, 1);
  state.replace("x", "draft-2");
  state.clearNavigation();
  assert.equal(state.capture(editor, details("navigation")).draft, null);
});

test("one action is captured once and prevented attempts are never confirmations", () => {
  const editor = new FrameworkInput("exact");
  const action = {defaultPrevented: true};
  const state = new EditorState();
  const capture = state.capture(editor, details(), action);
  assert.equal(capture.observation, "submit-attempt");
  assert.equal(capture.parent_id, null);
  assert.equal(state.capture(editor, details("duplicate"), action), null);
});

test("host adapters reject unsupported pages, generic buttons, and ambiguous editors", () => {
  assert.equal(detectSurface({hostname: "github.com", pathname: "/o/r/issues/7"}), null);
  assert.equal(detectSurface({hostname: "github.com", pathname: "/o/r/pull/7/files"}), "github-pr");
  assert.equal(detectSurface({hostname: "example.com", pathname: "/o/r/pull/7"}), null);

  const one = {querySelectorAll: () => [{id: "editor"}]};
  const many = {querySelectorAll: () => [{}, {}]};
  assert.deepEqual(nativeEditor({hostname: "github.com", pathname: "/o/r/pull/7"}, one), {id: "editor"});
  assert.equal(nativeEditor({hostname: "github.com", pathname: "/o/r/issues/7"}, one), null);
  assert.equal(nativeEditor({hostname: "github.com", pathname: "/o/r/pull/7"}, many), null);

  const generic = {closest: () => null};
  assert.equal(linearClickEditor({hostname: "linear.app", pathname: "/x"}, generic), null);
});

test("Linear SPA adapter requires explicit markers and one exact editor", () => {
  const editor = {id: "linear-editor"};
  const container = {querySelectorAll: () => [editor]};
  const submitter = {
    disabled: false,
    getAttribute: () => null,
    closest(selector) {
      if (selector.includes("submit-button")) return this;
      if (selector === "form") return null;
      if (selector.includes("composer")) return container;
      return null;
    },
  };
  const child = {closest: (selector) => selector.includes("submit-button") ? submitter : null};
  assert.equal(linearClickEditor({hostname: "linear.app", pathname: "/acme/issue/X-1"}, child), editor);
  submitter.closest = (selector) => selector === "form" ? {} : submitter;
  assert.equal(linearClickEditor({hostname: "linear.app", pathname: "/x"}, child), null);
});

test("assist accepts only recognized GitHub PR editors", () => {
  const location = {hostname: "github.com", pathname: "/o/r/pull/7"};
  const textarea = {matches: (selector) => selector.includes('textarea[name="comment[body]"]')};
  const search = {matches: () => false};
  const arbitraryEditable = {matches: () => false};
  assert.equal(assistEditor(location, textarea), textarea);
  assert.equal(assistEditor(location, search), null);
  assert.equal(assistEditor(location, arbitraryEditable), null);
  assert.equal(assistEditor({hostname: "github.com", pathname: "/o/r/issues/7"}, textarea), null);
});

test("assist accepts only the single recognized Linear editor in a composer", () => {
  const location = {hostname: "linear.app", pathname: "/acme/issue/X-1"};
  const container = {querySelectorAll: () => [editor]};
  const editor = {
    matches: (selector) => selector.includes("data-lexical-editor"),
    closest: (selector) => selector.includes("composer") ? container : null,
  };
  const arbitrary = {
    matches: (selector) => selector.includes("data-lexical-editor"),
    closest: () => null,
  };
  assert.equal(assistEditor(location, editor), editor);
  assert.equal(assistEditor(location, arbitrary), null);
  container.querySelectorAll = () => [editor, {}];
  assert.equal(assistEditor(location, editor), null);
});

test("surrounding context is bounded equally for native and contenteditable editors", () => {
  const text = "x".repeat(9000);
  const state = new EditorState();
  const input = new FrameworkInput(text);
  assert.equal(state.selectInput(input, 10, 20).surrounding_context.length, 8000);
  const range = {cloneRange() { return this; }};
  assert.equal(state.selectRange({isConnected: true}, range, "selected", text).surrounding_context.length, 8000);
  assert.equal(boundedContext(text), text.slice(0, 8000));
});

test("Chrome callback responses retain the channel until work settles", async () => {
  let resolve;
  let response;
  const pending = new Promise((done) => { resolve = done; });
  const returned = respondAsync(false, (value) => { response = value; }, () => pending);
  assert.equal(returned, true);
  assert.equal(response, undefined);
  resolve({ok: true});
  await new Promise((done) => setImmediate(done));
  assert.deepEqual(response, {ok: true});
});

test("Firefox listeners return the response Promise directly", async () => {
  let callbackCalled = false;
  const returned = respondAsync(true, () => { callbackCalled = true; }, () => ({selection: "x"}));
  assert.equal(typeof returned.then, "function");
  assert.deepEqual(await returned, {selection: "x"});
  assert.equal(callbackCalled, false);
});

test("one-way messages support Chrome callbacks and Firefox Promises", async () => {
  let chromeCallback;
  const chrome = {runtime: {
    lastError: undefined,
    sendMessage(_message, callback) { chromeCallback = callback; },
  }};
  sendOneWay(chrome, false, {type: "capture"});
  assert.equal(typeof chromeCallback, "function");
  chromeCallback();

  let promiseSent = false;
  const firefox = {runtime: {
    sendMessage() { promiseSent = true; return Promise.resolve(); },
  }};
  sendOneWay(firefox, true, {type: "capture"});
  await Promise.resolve();
  assert.equal(promiseSent, true);
});
