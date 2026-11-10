/** Run at most `concurrency` tasks at once; the rest wait in FIFO order. */
export function createLimiter(concurrency: number) {
  let active = 0;
  const queue: (() => void)[] = [];
  return async function run<T>(task: () => Promise<T>): Promise<T> {
    if (active >= concurrency) {
      // The finishing task hands its slot straight to us, so `active` is unchanged.
      const { promise, resolve } = Promise.withResolvers<void>();
      queue.push(resolve);
      await promise;
    } else {
      active++;
    }
    try {
      return await task();
    } finally {
      const waiter = queue.shift();
      if (waiter) waiter();
      else active--;
    }
  };
}
