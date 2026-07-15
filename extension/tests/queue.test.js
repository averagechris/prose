const assert = require("node:assert/strict");
const {test} = require("node:test");
const {createCaptureManager, disposition, drain} = require("../queue.js");

function memoryStorage(initial = {}) {
  const values = structuredClone(initial);
  return {
    values,
    async get(keys) {
      const wanted = Array.isArray(keys) ? keys : [keys];
      return Object.fromEntries(wanted.filter((key) => key in values).map((key) => [key, structuredClone(values[key])]));
    },
    async set(update) { Object.assign(values, structuredClone(update)); },
  };
}

test("network, 429, and 5xx failures retry while other 4xx are permanent", () => {
  assert.equal(disposition(new Error("offline")), "retry");
  assert.equal(disposition({status: 429}), "retry");
  assert.equal(disposition({status: 503}), "retry");
  assert.equal(disposition({status: 400}), "permanent");
  assert.equal(disposition({status: 409}), "permanent");
});

test("transient failure preserves stable IDs and later events for recovery", async () => {
  const queue = [{id: "a"}, {id: "b"}];
  const offline = await drain(queue, async () => { throw new Error("offline"); });
  assert.deepEqual(offline.remaining, queue);
  const sent = [];
  const recovered = await drain(offline.remaining, async (capture) => sent.push(capture.id));
  assert.deepEqual(sent, ["a", "b"]);
  assert.deepEqual(recovered.remaining, []);
  assert.deepEqual(recovered.completed, ["a", "b"]);
});

test("permanent failure is dead-lettered without poisoning later events", async () => {
  const sent = [];
  const result = await drain(
    [{id: "bad", content: "not diagnostic material"}, {id: "good"}],
    async (capture) => {
      if (capture.id === "bad") throw {status: 422};
      sent.push(capture.id);
    },
    () => "now",
  );
  assert.deepEqual(sent, ["good"]);
  assert.deepEqual(result.remaining, []);
  assert.deepEqual(result.dead, [{id: "bad", status: 422, at: "now"}]);
  assert.equal(JSON.stringify(result.dead).includes("diagnostic material"), false);
});

test("a hung delivery cannot block later persistence, and restart recovers stable IDs", async () => {
  const storage = memoryStorage();
  let startedA;
  const aStarted = new Promise((resolve) => { startedA = resolve; });
  const suspended = createCaptureManager({
    storage,
    send: async (capture) => {
      startedA(capture.id);
      return new Promise(() => {});
    },
  });

  await suspended.capture({id: "a", content: "first"});
  assert.equal(await aStarted, "a");
  await suspended.capture({id: "b", content: "second"});
  assert.deepEqual(storage.values.proseCaptureQueue.map((item) => item.id), ["a", "b"]);

  // A new worker has only local storage. The abandoned worker's unresolved
  // fetch cannot prevent it from recovering either capture.
  const delivered = [];
  const restarted = createCaptureManager({
    storage,
    send: async (capture) => delivered.push(structuredClone(capture)),
  });
  await restarted.flush();
  assert.deepEqual(delivered, [
    {id: "a", content: "first"},
    {id: "b", content: "second"},
  ]);
  assert.deepEqual(storage.values.proseCaptureQueue, []);
});

test("storage mutations stay serialized while delivery uses a separate chain", async () => {
  const storage = memoryStorage();
  const manager = createCaptureManager({storage, send: async () => {}});
  await Promise.all([
    manager.capture({id: "a"}),
    manager.capture({id: "b"}),
    manager.capture({id: "c"}),
  ]);
  await manager.flush();
  assert.deepEqual(storage.values.proseCaptureQueue, []);
});
