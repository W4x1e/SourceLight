/** A clipped-text finding: text cut off by a box that hides its overflow. */
interface Clip {
  text: string;
  where: string;
}

/**
 * Finds text the layout cuts off: an element whose own text overflows a
 * box that hides overflow, or text pushed outside an ancestor that hides it.
 * Deliberate `text-overflow: ellipsis` truncation is counted separately — it
 * is a design choice, not a clipping bug. Scrollers own their overflow and are
 * skipped. Also lists controls pushed below the window with no scroller to
 * reach them. Run it from the console, or a test driver, as `__clipAudit()`.
 *
 * Kept free of imports, so `scripts/audit-webview.mjs` can compile this file on
 * its own and inject it into the real app's WebView2.
 */
export function clipAudit() {
  const hides = (v: string) => v === "hidden" || v === "clip";
  const scrolls = (cs: CSSStyleDeclaration) =>
    ["auto", "scroll"].includes(cs.overflowX) || ["auto", "scroll"].includes(cs.overflowY);
  const own = (el: Element) =>
    [...el.childNodes]
      .filter((n) => n.nodeType === Node.TEXT_NODE)
      .map((n) => (n.textContent ?? "").trim())
      .join(" ")
      .trim();
  const clipped: Clip[] = [];
  let ellipsised = 0;

  for (const el of document.querySelectorAll("body *")) {
    const text = own(el);
    const r = el.getBoundingClientRect();
    if (!text || !r.width || !r.height) continue;
    const cs = getComputedStyle(el);
    const overX = el.scrollWidth > el.clientWidth + 1;
    const overY = el.scrollHeight > el.clientHeight + 1;
    if (overX && cs.textOverflow === "ellipsis") {
      ellipsised++;
      continue;
    }
    const clamp = cs.getPropertyValue("-webkit-line-clamp");
    const clamped = clamp !== "" && clamp !== "none";
    if ((overX && hides(cs.overflowX)) || (overY && hides(cs.overflowY) && !clamped)) {
      clipped.push({ text: text.slice(0, 60), where: `${el.tagName.toLowerCase()}.${el.className}` });
    }
  }

  for (const box of document.querySelectorAll("body *")) {
    const cs = getComputedStyle(box);
    if (!(hides(cs.overflowX) || hides(cs.overflowY)) || scrolls(cs)) continue;
    // A box that truncates with an ellipsis cuts its children on purpose, and
    // the first pass has already counted it.
    if (cs.textOverflow === "ellipsis") continue;
    const b = box.getBoundingClientRect();
    if (!b.width) continue;
    for (const child of box.querySelectorAll("*")) {
      const text = own(child);
      const c = child.getBoundingClientRect();
      if (!text || !c.width || !c.height) continue;
      let p = child.parentElement;
      let inScroller = false;
      while (p && p !== box) {
        if (scrolls(getComputedStyle(p))) {
          inScroller = true;
          break;
        }
        p = p.parentElement;
      }
      if (inScroller) continue;
      const cut = c.right > b.right + 1 || c.bottom > b.bottom + 1 || c.left < b.left - 1 || c.top < b.top - 1;
      const visible = c.bottom > b.top && c.top < b.bottom && c.right > b.left && c.left < b.right;
      if (cut && visible) {
        clipped.push({ text: text.slice(0, 60), where: `cut by ${box.tagName.toLowerCase()}.${box.className}` });
      }
    }
  }

  // Controls below the window that no scroller can bring into view — cut off
  // by the viewport rather than by a box, which the checks above cannot see.
  const unreachable: string[] = [];
  for (const el of document.querySelectorAll("button, input, select, label")) {
    const r = el.getBoundingClientRect();
    if (!r.width || !r.height || r.bottom <= innerHeight + 1) continue;
    let p = el.parentElement;
    let reachable = false;
    while (p) {
      if (scrolls(getComputedStyle(p)) && p.scrollHeight > p.clientHeight) {
        reachable = true;
        break;
      }
      p = p.parentElement;
    }
    if (!reachable) unreachable.push(((el as HTMLElement).innerText || el.getAttribute("title") || el.tagName).trim().slice(0, 40));
  }

  const seen = new Set<string>();
  return {
    size: `${innerWidth}x${innerHeight}`,
    ellipsised,
    unreachable,
    clipped: clipped.filter((c) => {
      const k = `${c.text}|${c.where}`;
      if (seen.has(k)) return false;
      seen.add(k);
      return true;
    }),
  };
}
