// Inspo: take design inspiration from another app on the phone. Pick an app, SwiftWin captures
// its screens (only tab-bar taps and scrolls), then shows the palette, type scale and screens,
// and can add InspoTheme.swift to the project or copy a restyle prompt for your AI agent (AI mode).
import { call } from "./api.js";
import { icon } from "./icons.js";
import { $, esc, toast, modal } from "./ui.js";
import { stagger, pulse } from "./motion.js";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const NOTE = `<p class="inspo-note dim">${icon("shield")}<span>Inspiration only: palette, spacing, type scale and patterns. Never copy the other app's logo, icons, artwork, name or exact screens.
  The captures may show your personal data; they stay on this PC in your project's <code>.inspo</code> folder (git-ignored).</span></p>`;

/** `ctx`: { ready() → error text or null, onApply(prompt), onThemeAdded() } */
export async function inspoDialog(ctx) {
  const why = ctx.ready();
  if (why) return toast(why, "info");
  const body = document.createElement("div");
  body.className = "inspo";
  let running = false;
  const close = modal({
    title: "Inspo",
    body,
    wide: true,
    onClose: () => running && call("inspo_cancel").catch(() => {}),
  });
  body.innerHTML = `<div class="scanning"><span class="spinner"></span> Reading the apps on your iPhone…</div>`;
  let apps;
  try {
    apps = (await call("inspo_apps")).apps;
  } catch (e) {
    body.innerHTML = `<div class="callout error">${esc(e.message)}</div>`;
    return;
  }
  body.innerHTML = `<p class="muted" style="margin:0 0 12px">Choose an app on your iPhone. SwiftWin opens it, looks around by itself (it only taps the tab bar and scrolls, so nothing gets bought, sent or deleted) and turns what it sees into a style guide for your app.</p>
    <input id="inspo-search" type="search" placeholder="Search your apps…" autocomplete="off" spellcheck="false">
    <div class="inspo-apps">${
      apps.length
        ? apps.map((a) => `<button class="inspo-app" data-id="${esc(a.bundle_id)}" data-q="${esc((a.name + " " + a.bundle_id).toLowerCase())}"><b>${esc(a.name)}</b><span class="dim">${esc(a.bundle_id)}</span></button>`).join("")
        : `<div class="dim">No apps of your own on the phone yet: install one from the App Store first.</div>`
    }</div>
    <label class="inspo-deep"><input type="checkbox" id="inspo-deep"><span><b>Deep capture</b> <span class="dim">· also opens every button and row on each tab and comes back (up to 40 screens, several minutes). It still never taps anything that buys, sends, posts, deletes, signs out, likes or adds.</span></span></label>${NOTE}`;
  stagger(body.querySelectorAll(".inspo-app"));
  // Plain cards (no Liquid Glass): ~100 refracting cards made the dialog lag.
  const search = $("inspo-search");
  search.oninput = () => {
    const q = search.value.trim().toLowerCase();
    body.querySelectorAll(".inspo-app").forEach((b) => b.classList.toggle("hidden", !!q && !b.dataset.q.includes(q)));
  };
  search.onkeydown = (e) => {
    if (e.key !== "Enter") return;
    const first = body.querySelector(".inspo-app:not(.hidden)");
    if (first) first.click();
  };
  setTimeout(() => search.focus(), 50);
  body.querySelectorAll(".inspo-app").forEach((b) => (b.onclick = () => capture(b.dataset.id, b.querySelector("b").textContent)));

  async function capture(id, name) {
    const deep = $("inspo-deep").checked;
    try {
      await call("inspo_start", { bundle_id: id, deep });
    } catch (e) {
      return toast(e.message, "error");
    }
    running = true;
    body.innerHTML = `<div class="inspo-progress"><span class="spinner"></span><div><b>${deep ? "Deep-exploring" : "Exploring"} ${esc(name)}…</b><div id="inspo-msg" class="dim">Starting</div></div>
      <button id="inspo-cancel" class="btn small ghost">Stop</button></div>${NOTE}`;
    $("inspo-cancel").onclick = () => call("inspo_cancel").catch(() => {});
    let st;
    for (;;) {
      await sleep(400);
      if (!body.isConnected) return;
      st = await call("inspo_status").catch(() => ({ running: true }));
      if (!st.running) break;
      if (st.message) $("inspo-msg").textContent = `${st.message} · ${st.screens} of up to ${st.max || 12} screens`;
    }
    running = false;
    if (st.error) {
      body.innerHTML = `<div class="callout error">${esc(st.error)}</div>`;
      return;
    }
    show(st.result);
  }

  function show(r) {
    const g = r.guide;
    const p = g.palette;
    const chip = (hex, label) => `<div class="inspo-chip"><span style="background:${esc(hex)}"></span><b>${esc(label)}</b><code>${esc(hex)}</code></div>`;
    const roles = [["Background", p.background], ["Surface", p.surface], ["Text", p.text], ...(p.accent ? [["Accent", p.accent]] : [])];
    body.innerHTML = `<div class="inspo-result">
      <div class="inspo-side">
        <h3>Palette</h3><div class="inspo-chips">${roles.map(([l, h]) => chip(h, l)).join("")}</div>
        <div class="inspo-swatches">${p.dominant.map((s) => `<span title="${esc(s.hex)} · ${Math.round(s.share * 100)}%" style="background:${esc(s.hex)};flex:${Math.max(s.share, 0.03)}"></span>`).join("")}</div>
        <h3>Type scale</h3><div class="inspo-type">${g.type_scale.map((t) => `<div><span style="font-size:${Math.min(t.size, 34)}px">Aa</span><code>${t.size}pt</code><span class="dim">${t.count}×</span></div>`).join("") || `<span class="dim">No text found</span>`}</div>
        <h3>Spacing</h3><div class="muted">Padding ${g.spacing.screen_padding} · stack ${g.spacing.stack}${g.spacing.row_height ? ` · rows ${g.spacing.row_height}` : ""}${g.spacing.corner_radius != null ? ` · corners ${g.spacing.corner_radius}` : ""}</div>
        <h3>Patterns</h3><div class="muted">${esc(g.components.map((c) => c.name).join(", ") || "none detected")}</div>
      </div>
      <div class="inspo-grid">${g.screens.map((s) => `<div class="inspo-shot" data-path="${esc(r.dir)}/${esc(s.image)}"></div>`).join("")}</div>
    </div>
    ${r.cancelled ? `<div class="callout">Stopped early: the guide uses the ${g.screens.length} screen(s) captured so far.</div>` : ""}
    ${NOTE}
    <div class="form-actions"><span class="dim grow">Saved in <code>${esc(r.dir)}</code></span>
      <button id="inspo-theme" class="btn">${icon("plus")}Add theme to my project</button>
      <button id="inspo-claude" class="btn primary">${icon("sparkles")}Copy prompt for your AI agent</button></div>`;
    stagger(body.querySelectorAll(".inspo-chip, .inspo-type > div, .inspo-shot"));
    body.querySelectorAll(".inspo-shot").forEach(async (d) => {
      try {
        const img = await call("inspo_image", { path: d.dataset.path });
        d.innerHTML = `<img alt="Captured screen" src="data:image/png;base64,${img.data}">`;
      } catch {
        d.classList.add("broken");
      }
    });
    $("inspo-theme").onclick = async (ev) => {
      const btn = ev.currentTarget; // null once the handler awaits
      try {
        const t = await call("inspo_add_theme", { dir: r.dir });
        pulse(btn);
        toast(`Added ${t.path}: use InspoTheme.background, .accent, .title… in your views`, "success", 6000);
        ctx.onThemeAdded?.();
      } catch (e) {
        toast(e.message, "error");
      }
    };
    $("inspo-claude").onclick = () => {
      close();
      ctx.onApply(r.prompt);
    };
  }
}
