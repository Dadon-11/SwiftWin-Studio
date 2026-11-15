// Small UI helpers: escaping, toasts, modals, relative time.
import { icon } from "./icons.js";
import { DUR_OUT, stagger } from "./motion.js";

// The control the user last pressed: popovers grow out of it and shrink back into it.
let lastTrigger = null;
let lastTriggerAt = 0;
document.addEventListener(
  "pointerdown",
  (e) => {
    lastTrigger = e.target.closest?.("button, [data-action], .tile, .project-btn") || null;
    lastTriggerAt = Date.now();
  },
  true,
);

export const $ = (id) => document.getElementById(id);

export function esc(s) {
  return String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}

export function toast(text, kind = "info", ms = 4000) {
  const box = $("toasts");
  const el = document.createElement("div");
  el.className = `toast ${kind}`;
  el.dataset.glass = "bend press"; // Liquid Glass capsule (see vendor/glisse)
  const ic = { success: "check", error: "alert", info: "info" }[kind] || "info";
  el.innerHTML = `${icon(ic)}<span>${esc(text)}</span>`;
  box.appendChild(el);
  requestAnimationFrame(() => el.classList.add("show"));
  const close = () => {
    el.classList.remove("show");
    setTimeout(() => el.remove(), DUR_OUT + 10);
  };
  el.onclick = close;
  if (ms) setTimeout(close, kind === "error" ? Math.max(ms, 7000) : ms);
  return el;
}

/** Open a modal. `body` is an Element; returns a close function. */
export function modal({ title, body, wide = false, onClose }) {
  const back = document.createElement("div");
  back.className = "modal-back";
  back.innerHTML = `<div class="modal ${wide ? "wide" : ""}" role="dialog" aria-modal="true" data-glass>
      <div class="modal-head"><h2>${esc(title)}</h2><button class="icon-btn modal-x" title="Close (Esc)">${icon("x")}</button></div>
      <div class="modal-body"></div></div>`;
  back.querySelector(".modal-body").appendChild(body);
  document.body.appendChild(back);
  // Grow out of the control that opened it (and shrink back into it on close).
  const box = back.querySelector(".modal");
  const trigger = Date.now() - lastTriggerAt < 1500 && lastTrigger?.isConnected ? lastTrigger : null;
  if (trigger) {
    const t = trigger.getBoundingClientRect();
    const m = box.getBoundingClientRect(); // its centre does not move with the scale
    const left = m.left + m.width / 2 - box.offsetWidth / 2;
    const top = m.top + m.height / 2 - box.offsetHeight / 2;
    box.style.transformOrigin = `${t.left + t.width / 2 - left}px ${t.top + t.height / 2 - top}px`;
  }
  requestAnimationFrame(() => {
    back.classList.add("show");
    stagger(back.querySelectorAll(".modal-body .field, .modal-body .form-actions, .modal-body .tabs-row"), { from: 60 });
  });
  const close = () => {
    document.removeEventListener("keydown", onKey, true);
    back.classList.remove("show");
    back.classList.add("closing");
    setTimeout(() => back.remove(), DUR_OUT + 10);
    onClose?.();
  };
  const onKey = (e) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      close();
    }
  };
  document.addEventListener("keydown", onKey, true);
  back.querySelector(".modal-x").onclick = close;
  back.addEventListener("mousedown", (e) => e.target === back && close());
  return close;
}

export function ago(iso) {
  const s = Math.max(0, Math.round((Date.now() - Date.parse(iso)) / 1000));
  if (s < 5) return "just now";
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  return `${Math.floor(s / 3600)}h ago`;
}

export function store(key, value) {
  try {
    if (value === undefined) return JSON.parse(localStorage.getItem("swiftwin." + key));
    localStorage.setItem("swiftwin." + key, JSON.stringify(value));
  } catch {
    return undefined;
  }
}
