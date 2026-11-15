// Motion: small, consistent animations for the whole app.
//
// Rules (durations and curves live in style.css as --ease-* / --dur-* tokens):
// entrances ease out, moves ease in-out, exits are 20% shorter than entrances,
// no bounce on everyday UI, presses respond instantly, lists stagger by 30 ms,
// popovers start at 0.95 and grow out of (and shrink back into) what opened them.
// Everything is skipped when Windows asks for less motion.

export const EASE_OUT = "cubic-bezier(.23, 1, .32, 1)";
export const EASE_IN_OUT = "cubic-bezier(.77, 0, .175, 1)";
export const EASE_IN = "cubic-bezier(.55, 0, 1, .45)";
export const DUR_IN = 220;
export const DUR_OUT = Math.round(DUR_IN * 0.8); // exits 20% shorter

export const reduced = () => window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;

/** Fade rows up one after another, 30 ms apart (at most ~10 steps, so long lists stay quick). */
export function stagger(elements, { step = 30, from = 0, y = 4 } = {}) {
  if (reduced()) return;
  [...elements].forEach((el, i) => {
    el.animate(
      [
        { opacity: 0, transform: `translateY(${y}px)` },
        { opacity: 1, transform: "none" },
      ],
      { duration: DUR_IN, delay: from + Math.min(i, 10) * step, easing: EASE_OUT, fill: "backwards" },
    );
  });
}

/** A short pulse that confirms something worked. */
export function pulse(el, color = "rgba(60, 207, 110, 0.55)") {
  if (!el || reduced()) return;
  el.animate(
    [
      { boxShadow: `0 0 0 0 ${color}`, transform: "scale(1)" },
      { boxShadow: `0 0 0 6px ${color.replace(/[\d.]+\)$/, "0.18)")}`, transform: "scale(1.03)", offset: 0.35 },
      { boxShadow: "0 0 0 10px rgba(0, 0, 0, 0)", transform: "scale(1)" },
    ],
    { duration: 520, easing: EASE_OUT },
  );
}

/** Swap an icon (an element holding one <svg>) with a crossfade and a little blur. */
export function swapIcon(holder, html) {
  const old = holder.querySelector("svg");
  if (!old || reduced()) {
    holder.innerHTML = html;
    return;
  }
  const tmp = document.createElement("span");
  tmp.innerHTML = html;
  const next = tmp.firstElementChild;
  if (!next) return;
  holder.style.position ||= "relative";
  old.style.position = "absolute";
  holder.appendChild(next);
  const out = { duration: DUR_OUT, easing: EASE_IN, fill: "forwards" };
  old.animate([{ opacity: 1, filter: "blur(0)", scale: "1" }, { opacity: 0, filter: "blur(4px)", scale: "0.6" }], out).onfinish = () =>
    old.remove();
  next.animate([{ opacity: 0, filter: "blur(4px)", scale: "0.6" }, { opacity: 1, filter: "blur(0)", scale: "1" }], {
    duration: DUR_IN,
    easing: EASE_OUT,
  });
}

/**
 * Move an absolutely positioned element (left/top in %) along a gentle arc, easing in and out.
 * `from` / `to` are {x, y} fractions of its container.
 */
export function arcMove(el, from, to, duration = 450) {
  const place = (p) => {
    el.style.left = p.x * 100 + "%";
    el.style.top = p.y * 100 + "%";
  };
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const dist = Math.hypot(dx, dy);
  if (reduced() || dist < 0.02) {
    el.getAnimations().forEach((a) => a.cancel());
    place(to);
    return;
  }
  // Control point: the midpoint pushed sideways by 18% of the distance (always bending the same way).
  const c = { x: from.x + dx / 2 - dy * 0.18, y: from.y + dy / 2 + dx * 0.18 };
  const frames = [];
  for (let i = 0; i <= 16; i++) {
    const t = i / 16;
    const u = 1 - t;
    const x = u * u * from.x + 2 * u * t * c.x + t * t * to.x;
    const y = u * u * from.y + 2 * u * t * c.y + t * t * to.y;
    frames.push({ left: x * 100 + "%", top: y * 100 + "%" });
  }
  el.getAnimations().forEach((a) => a.cancel());
  place(to);
  el.animate(frames, { duration, easing: EASE_IN_OUT });
}

/**
 * A sliding indicator behind the active item of a group (segmented control, tabs).
 * The same element moves between items (one "layout id"), and stretches 12% while it travels.
 * Returns `update()`; call it after changing which item is active.
 */
export function slidingIndicator(container, itemSelector) {
  const ind = document.createElement("span");
  ind.className = "slide-ind";
  ind.setAttribute("aria-hidden", "true");
  container.classList.add("has-slide-ind");
  container.prepend(ind);
  let shown = false;
  const update = (animate = true) => {
    const active = container.querySelector(`${itemSelector}.active`);
    if (!active) {
      ind.style.opacity = "0";
      return;
    }
    const x = active.offsetLeft;
    const w = active.offsetWidth;
    const prev = ind.style.transform;
    const next = `translateX(${x}px)`;
    const moving = shown && animate && prev && prev !== next && !reduced();
    ind.style.width = w + "px";
    ind.style.height = active.offsetHeight + "px";
    ind.style.top = active.offsetTop + "px";
    ind.style.opacity = "1";
    ind.classList.toggle("instant", !moving);
    ind.style.transform = next;
    if (moving) {
      ind.animate([{ scale: "1 1" }, { scale: "1.12 0.94", offset: 0.45 }, { scale: "1 1" }], {
        duration: 280,
        easing: EASE_IN_OUT,
      });
    }
    shown = true;
  };
  // Follow size changes of the group and of each item (e.g. a tab whose count appears).
  const ro = new ResizeObserver(() => update(false));
  ro.observe(container);
  container.querySelectorAll(itemSelector).forEach((item) => ro.observe(item));
  requestAnimationFrame(() => update(false));
  return update;
}

/**
 * Run `change` as a view transition: old and new layouts overlap, named elements (the phone)
 * glide to their new place, and the new layout is revealed in a circle growing from `from`.
 */
export function transition(change, from) {
  if (!document.startViewTransition || reduced()) {
    change();
    return;
  }
  if (from) {
    const r = from.getBoundingClientRect();
    const root = document.documentElement.style;
    root.setProperty("--vt-x", r.left + r.width / 2 + "px");
    root.setProperty("--vt-y", r.top + r.height / 2 + "px");
  }
  document.startViewTransition(change);
}

/** Rubber band: how far past a bound to move for `over` px of pointer travel. */
export function rubber(over, dim = 140) {
  return (1 - 1 / ((over * 0.55) / dim + 1)) * dim;
}
