import { test, expect } from 'bun:test';
import { onCleanupFinished, CLEANUP_EVENT } from './cleanupEvents';

// A fake `listen` we control: the test resolves or rejects its promise,
// calls the captured handler as the "event", and counts remove calls.
function makeFake() {
  const fake = {
    eventNames: [] as string[],
    handler: null as null | (() => void),
    removeCalls: 0,
    resolve: (() => {}) as () => void,
    reject: (() => {}) as (e: unknown) => void,
    listen(event: string, handler: () => void): Promise<() => void> {
      fake.eventNames.push(event);
      fake.handler = handler;
      return new Promise<() => void>((res, rej) => {
        fake.resolve = () =>
          res(() => {
            fake.removeCalls += 1;
          });
        fake.reject = rej;
      });
    },
    // Fires the event even after stop: the fake never removes the handler itself.
    fire() {
      fake.handler?.();
    },
  };
  return fake;
}

// Let chained .then callbacks run.
async function settle() {
  for (let i = 0; i < 5; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

test('CLEANUP_EVENT is assistant-cleanup', () => {
  expect(CLEANUP_EVENT).toBe('assistant-cleanup');
});

test('listen is called with the CLEANUP_EVENT name', async () => {
  const fake = makeFake();
  onCleanupFinished(fake.listen, () => {});
  fake.resolve();
  await settle();
  expect(fake.eventNames).toEqual(['assistant-cleanup']);
});

test('the event firing once runs onChange once', async () => {
  const fake = makeFake();
  let changes = 0;
  onCleanupFinished(fake.listen, () => {
    changes += 1;
  });
  fake.resolve();
  await settle();
  fake.fire();
  expect(changes).toBe(1);
});

test('the event firing twice runs onChange twice', async () => {
  const fake = makeFake();
  let changes = 0;
  onCleanupFinished(fake.listen, () => {
    changes += 1;
  });
  fake.resolve();
  await settle();
  fake.fire();
  fake.fire();
  expect(changes).toBe(2);
});

test('worked example: two events, stop, a third event does not reach onChange', async () => {
  const fake = makeFake();
  let changes = 0;
  const stop = onCleanupFinished(fake.listen, () => {
    changes += 1;
  });
  fake.resolve();
  await settle();
  fake.fire();
  fake.fire();
  expect(changes).toBe(2);
  stop();
  fake.fire();
  expect(changes).toBe(2);
});

test('stop after listen resolved calls remove at once and later events are ignored', async () => {
  const fake = makeFake();
  let changes = 0;
  const stop = onCleanupFinished(fake.listen, () => {
    changes += 1;
  });
  fake.resolve();
  await settle();
  expect(fake.removeCalls).toBe(0);
  stop();
  expect(fake.removeCalls).toBe(1);
  fake.fire();
  expect(changes).toBe(0);
});

test('stop before listen resolves removes as soon as it resolves and later events are ignored', async () => {
  const fake = makeFake();
  let changes = 0;
  const stop = onCleanupFinished(fake.listen, () => {
    changes += 1;
  });
  stop();
  await settle();
  expect(fake.removeCalls).toBe(0); // nothing to remove yet
  fake.resolve();
  await settle();
  expect(fake.removeCalls).toBe(1);
  fake.fire();
  expect(changes).toBe(0);
});

test('an event delivered before listen resolves and after stop does not run onChange', async () => {
  const fake = makeFake();
  let changes = 0;
  const stop = onCleanupFinished(fake.listen, () => {
    changes += 1;
  });
  stop();
  fake.fire();
  fake.resolve();
  await settle();
  fake.fire();
  expect(changes).toBe(0);
});

test('listen rejecting does not throw, leaves no unhandled rejection, and stop is safe', async () => {
  const unhandled: unknown[] = [];
  const onUnhandled = (reason: unknown) => {
    unhandled.push(reason);
  };
  process.on('unhandledRejection', onUnhandled);
  try {
    const fake = makeFake();
    let stop: () => void = () => {};
    expect(() => {
      stop = onCleanupFinished(fake.listen, () => {});
    }).not.toThrow();
    fake.reject(new Error('listen failed'));
    await settle();
    expect(typeof stop).toBe('function');
    expect(() => stop()).not.toThrow();
    await settle();
    expect(unhandled).toEqual([]);
    expect(fake.removeCalls).toBe(0);
  } finally {
    process.off('unhandledRejection', onUnhandled);
  }
});

test('stop called before listen rejects does not throw either', async () => {
  const fake = makeFake();
  const stop = onCleanupFinished(fake.listen, () => {});
  expect(() => stop()).not.toThrow();
  fake.reject(new Error('listen failed'));
  await settle();
  expect(fake.removeCalls).toBe(0);
});
