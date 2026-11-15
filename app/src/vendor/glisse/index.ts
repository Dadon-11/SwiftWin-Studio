// SwiftWin's entry to glisse (Liquid Glass for the web, MIT, see README.md): glass on plain
// elements instead of React components.
//
//   <div data-glass>                 frost and the lit rim (panels, cards)
//   <button data-glass="bend press"> also bends what's behind it (Chromium, so WebView2) and
//                                    springs when pressed, as iOS 26 glass does
//
// `autoGlass()` attaches it to every `[data-glass]` element, including ones added later
// (toasts, dialogs, device cards), and detaches it from removed ones.

import { attachPress } from "./press";
import { attachRefraction } from "./refraction";
import { insertStyles } from "./styles";
import css from "./styles.css?inline";

insertStyles(css);

const attached = new Map<HTMLElement, () => void>();

/** Makes `el` Liquid Glass. Returns a function that removes it again. */
export function glass(el: HTMLElement, { bend = false, press = false } = {}): () => void {
  attached.get(el)?.();
  el.classList.add("lg-glass");
  const stops = [attachRefraction(el, { bendsBackdrop: bend })];
  if (press) stops.push(attachPress(el));
  const stop = () => {
    stops.forEach((s) => s());
    attached.delete(el);
  };
  attached.set(el, stop);
  return stop;
}

function attach(el: HTMLElement): void {
  const flags = (el.dataset.glass || "").split(/\s+/);
  glass(el, { bend: flags.includes("bend"), press: flags.includes("press") });
}

function each(node: Node, fn: (el: HTMLElement) => void): void {
  if (!(node instanceof HTMLElement)) return;
  if (node.matches("[data-glass]")) fn(node);
  node.querySelectorAll<HTMLElement>("[data-glass]").forEach(fn);
}

/** Glass for every `[data-glass]` element under `root`, now and as they come and go. */
export function autoGlass(root: HTMLElement = document.body): void {
  each(root, attach);
  new MutationObserver((records) => {
    for (const r of records) {
      r.removedNodes.forEach((n) => each(n, (el) => !el.isConnected && attached.get(el)?.()));
      r.addedNodes.forEach((n) => each(n, (el) => !attached.has(el) && attach(el)));
    }
  }).observe(root, { childList: true, subtree: true });
}
