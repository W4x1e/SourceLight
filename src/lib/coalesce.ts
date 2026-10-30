/**
 * Passes on the latest value for each key at most every `ms`.
 *
 * A pass reports progress per photo, and each report updated the store: the
 * Activity panel and Cull re-rendered for every one, thousands in a run. Only
 * the newest value for each key is kept until the next flush. A final value
 * (`last`) goes through at once and drops anything waiting, so a stale report
 * can never land after it.
 */
export function coalescer<T>(apply: (key: string, value: T) => void, ms = 100) {
  const waiting = new Map<string, T>();
  let timer: ReturnType<typeof setTimeout> | null = null;
  const flush = () => {
    timer = null;
    const due = [...waiting];
    waiting.clear();
    for (const [key, value] of due) apply(key, value);
  };
  return (key: string, value: T, last = false) => {
    if (last) {
      waiting.delete(key);
      apply(key, value);
      return;
    }
    waiting.set(key, value);
    timer ??= setTimeout(flush, ms);
  };
}
