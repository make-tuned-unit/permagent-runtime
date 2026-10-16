/** Bound a read even when a provider ignores AbortSignal. Late settlements
 * cannot change the result; callers still guard against newer requests. */
export async function readWithDeadline<T>(read: (signal: AbortSignal) => Promise<T>, ms = 20_000): Promise<T> {
  const controller = new AbortController();
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      Promise.resolve().then(() => read(controller.signal)),
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => {
          controller.abort();
          reject(new Error('The request timed out. Please try again.'));
        }, ms);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}
