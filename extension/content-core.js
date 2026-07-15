(function (root, factory) {
  const value = factory();
  root.ProseContentCore = value;
  if (typeof module === "object" && module.exports) module.exports = value;
})(globalThis, function () {
  "use strict";

  const githubPath = /^\/[^/]+\/[^/]+\/pull\/\d+(?:\/|$)/;
  const githubEditors = [
    'textarea[name="comment[body]"]',
    'textarea[name="pull_request_review[body]"]',
    'textarea[data-testid="review-comment-textarea"]',
  ].join(", ");
  const linearEditors = '[data-lexical-editor="true"][contenteditable="true"]';
  const linearContainers = [
    '[data-testid="composer"]',
    '[data-testid="comment-composer"]',
    '[data-testid="issue-comment-composer"]',
  ].join(", ");
  const linearSubmitters = [
    'button[data-testid="composer-submit-button"]',
    'button[data-testid="comment-submit-button"]',
    '[role="button"][data-testid="composer-submit-button"]',
  ].join(", ");
  const surroundingContextLimit = 8000;

  function detectSurface(locationLike) {
    if (locationLike.hostname === "github.com" && githubPath.test(locationLike.pathname)) {
      return "github-pr";
    }
    if (locationLike.hostname === "linear.app") return "linear";
    return null;
  }

  function exactOne(rootNode, selector) {
    const found = Array.from(rootNode?.querySelectorAll?.(selector) ?? []);
    return found.length === 1 ? found[0] : null;
  }

  function nativeEditor(locationLike, form) {
    const surface = detectSurface(locationLike);
    if (!surface || !form?.querySelectorAll) return null;
    if (surface === "github-pr") return exactOne(form, githubEditors);
    return exactOne(form, linearEditors);
  }

  function linearClickEditor(locationLike, target) {
    if (detectSurface(locationLike) !== "linear") return null;
    const submitter = target?.closest?.(linearSubmitters);
    if (!submitter || submitter.disabled || submitter.getAttribute?.("aria-disabled") === "true") return null;
    // A form-associated control is handled only by the native submit event.
    if (submitter.closest?.("form")) return null;
    const container = submitter.closest?.(linearContainers);
    return container ? exactOne(container, linearEditors) : null;
  }

  function assistEditor(locationLike, candidate) {
    const surface = detectSurface(locationLike);
    if (!surface || !candidate?.matches) return null;
    if (surface === "github-pr") return candidate.matches(githubEditors) ? candidate : null;
    if (!candidate.matches(linearEditors)) return null;
    const container = candidate.closest?.(linearContainers);
    return container && exactOne(container, linearEditors) === candidate ? candidate : null;
  }

  function editorText(editor) {
    return "value" in editor ? editor.value : (editor.innerText ?? editor.textContent ?? "");
  }

  function boundedContext(value) {
    return String(value).slice(0, surroundingContextLimit);
  }

  function respondAsync(promiseResponses, sendResponse, work) {
    const pending = Promise.resolve().then(work);
    if (promiseResponses) return pending;
    pending.then(
      (value) => sendResponse(value),
      () => sendResponse({ok: false}),
    );
    // Chrome requires this exact sentinel to retain an asynchronous channel.
    return true;
  }

  function sendOneWay(api, promiseResponses, message) {
    try {
      if (promiseResponses) {
        const pending = api.runtime.sendMessage(message);
        pending?.catch?.(() => undefined);
      } else {
        api.runtime.sendMessage(message, () => void api.runtime.lastError);
      }
    } catch (_) {
      // The content script intentionally retains no copy of authored material.
    }
  }

  function inputEvent(type, candidate) {
    if (typeof InputEvent === "function") {
      return new InputEvent(type, {
        bubbles: true,
        cancelable: type === "beforeinput",
        inputType: "insertText",
        data: candidate,
      });
    }
    return new Event(type, {bubbles: true, cancelable: type === "beforeinput"});
  }

  function setNativeValue(element, value) {
    let prototype = Object.getPrototypeOf(element);
    while (prototype) {
      const setter = Object.getOwnPropertyDescriptor(prototype, "value")?.set;
      if (setter) {
        setter.call(element, value);
        return;
      }
      prototype = Object.getPrototypeOf(prototype);
    }
    element.value = value;
  }

  class EditorState {
    constructor() {
      this.pending = null;
      this.drafts = new Map();
      this.actions = new WeakSet();
    }

    selectInput(element, start, end) {
      this.pending = {kind: "input", editor: element, start, end};
      return {selection: element.value.slice(start, end), surrounding_context: boundedContext(element.value)};
    }

    selectRange(editor, range, selection, context) {
      this.pending = {kind: "range", editor, range: range.cloneRange ? range.cloneRange() : range};
      return {selection, surrounding_context: boundedContext(context)};
    }

    replace(candidate, draft) {
      const pending = this.pending;
      this.pending = null;
      if (!pending?.editor?.isConnected || typeof candidate !== "string") return false;
      try {
        if (pending.editor.dispatchEvent(inputEvent("beforeinput", candidate)) === false) return false;
        if (pending.kind === "input") {
          const {editor, start, end} = pending;
          const replacement = `${editor.value.slice(0, start)}${candidate}${editor.value.slice(end)}`;
          setNativeValue(editor, replacement);
          editor.setSelectionRange?.(start + candidate.length, start + candidate.length);
          editor.dispatchEvent(inputEvent("input", candidate));
          if (editor.value !== replacement) return false;
        } else {
          const {editor, range} = pending;
          range.deleteContents();
          range.insertNode(editor.ownerDocument.createTextNode(candidate));
          range.collapse?.(false);
          editor.dispatchEvent(inputEvent("input", candidate));
        }
      } catch (_) {
        return false;
      }
      this.drafts.set(pending.editor, {candidate, draft});
      return true;
    }

    capture(editor, details, action) {
      if (!editor?.isConnected || (action && this.actions.has(action))) return null;
      if (action) this.actions.add(action);
      const content = editorText(editor);
      if (content.trim().length === 0) return null;
      const association = this.drafts.get(editor);
      return {
        id: details.id,
        observation: "submit-attempt",
        parent_id: null,
        surface: details.surface,
        url: details.url,
        content,
        draft: association && association.candidate === content ? association.draft : null,
        metadata: {source: details.source},
      };
    }

    invalidate(editor) {
      if (editor) this.drafts.delete(editor);
    }

    clearDisconnected() {
      for (const editor of this.drafts.keys()) {
        if (!editor.isConnected) this.drafts.delete(editor);
      }
      if (this.pending && !this.pending.editor.isConnected) this.pending = null;
    }

    clearNavigation() {
      this.pending = null;
      this.drafts.clear();
      this.actions = new WeakSet();
    }
  }

  return {
    EditorState,
    assistEditor,
    boundedContext,
    detectSurface,
    editorText,
    linearClickEditor,
    nativeEditor,
    respondAsync,
    sendOneWay,
    selectors: {githubEditors, linearContainers, linearEditors, linearSubmitters},
    surroundingContextLimit,
  };
});
