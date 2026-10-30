/**
 * `localStorage` that writes a key only when its value changed.
 *
 * The store's persist middleware writes the saved preferences after every
 * change to any state, and most changes are to state that isn't saved: each
 * progress event of a scan or an analysis pass, each verdict, each hover. A
 * write is synchronous, on the thread that draws the window, so a pass of
 * thousands of photos made thousands of them, all with the same value.
 */
export function changedOnlyStorage(backing: Storage): Pick<Storage, "getItem" | "setItem" | "removeItem"> {
  const written = new Map<string, string>();
  return {
    getItem: (key) => {
      const value = backing.getItem(key);
      if (value !== null) written.set(key, value);
      return value;
    },
    setItem: (key, value) => {
      if (written.get(key) === value) return;
      backing.setItem(key, value);
      written.set(key, value);
    },
    removeItem: (key) => {
      written.delete(key);
      backing.removeItem(key);
    },
  };
}
