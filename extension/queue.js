(function (root, factory) {
  const value = factory();
  root.ProseQueue = value;
  if (typeof module === "object" && module.exports) module.exports = value;
})(globalThis, function () {
  "use strict";

  function disposition(error) {
    const status = Number(error?.status ?? 0);
    if (status === 429 || status >= 500 || status === 0) return "retry";
    if (status >= 400 && status < 500) return "permanent";
    return "retry";
  }

  async function drain(queue, send, now = () => new Date().toISOString()) {
    const remaining = [];
    const dead = [];
    const completed = [];
    for (let index = 0; index < queue.length; index += 1) {
      const capture = queue[index];
      try {
        await send(capture);
        completed.push(capture.id);
      } catch (error) {
        if (disposition(error) === "permanent") {
          dead.push({id: capture.id, status: Number(error.status), at: now()});
          completed.push(capture.id);
          continue;
        }
        remaining.push(...queue.slice(index));
        break;
      }
    }
    return {remaining, dead, completed};
  }

  function createCaptureManager({
    storage,
    send,
    queueKey = "proseCaptureQueue",
    deadKey = "proseCaptureDeadLetters",
    deadLimit = 25,
    now,
  }) {
    let storageWork = Promise.resolve();
    let deliveryWork = Promise.resolve();

    function mutateStorage(work) {
      const result = storageWork.then(work, work);
      // A failed storage operation must not poison later persistence attempts.
      storageWork = result.catch(() => undefined);
      return result;
    }

    function append(capture) {
      return mutateStorage(async () => {
        const stored = await storage.get(queueKey);
        const queue = Array.isArray(stored[queueKey]) ? stored[queueKey] : [];
        if (!queue.some((item) => item.id === capture.id)) queue.push(capture);
        await storage.set({[queueKey]: queue});
      });
    }

    async function deliverSnapshot() {
      const snapshot = await mutateStorage(async () => {
        const stored = await storage.get(queueKey);
        return Array.isArray(stored[queueKey]) ? stored[queueKey] : [];
      });
      if (snapshot.length === 0) return;

      // No storage lock is held while network delivery is in progress. A later
      // capture can therefore be persisted even if this send never settles.
      const result = await drain(snapshot, send, now);
      const completed = new Set(result.completed);
      await mutateStorage(async () => {
        const stored = await storage.get([queueKey, deadKey]);
        const latest = Array.isArray(stored[queueKey]) ? stored[queueKey] : [];
        const priorDead = Array.isArray(stored[deadKey]) ? stored[deadKey] : [];
        await storage.set({
          [queueKey]: latest.filter((capture) => !completed.has(capture.id)),
          [deadKey]: [...priorDead, ...result.dead].slice(-deadLimit),
        });
      });
    }

    function flush() {
      const result = deliveryWork.then(deliverSnapshot, deliverSnapshot);
      deliveryWork = result.catch(() => undefined);
      return result;
    }

    function capture(captureValue) {
      const persisted = append(captureValue);
      // Delivery has its own chain. The caller acknowledges only persistence,
      // while retries remain best-effort background work.
      persisted.then(() => {
        flush().catch(() => undefined);
      }, () => undefined);
      return persisted;
    }

    return {append, capture, flush};
  }

  return {createCaptureManager, disposition, drain};
});
