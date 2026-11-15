// One entry point to the Rust backend (AppCore::dispatch).
// Inside the Tauri app it is the `core` command; in a browser it is POST /api/<cmd>
// served by `swiftwin ui` (development and UI tests), authenticated with a per-run token.
const inTauri = typeof window !== "undefined" && !!window.__TAURI_INTERNALS__;
let tokenPromise = null;

/** The phone page in the Claude app's Browser pane (`#phone&pair=<code>`) trades its one-time code
 *  for the token and keeps it for this tab (reloads); `swiftwin ui` hands its token out at /token. */
async function token() {
  if (!tokenPromise) {
    const code = new URLSearchParams(location.hash.slice(1)).get("pair");
    if (code) history.replaceState(null, "", location.pathname + "#phone"); // the code works once anyway
    tokenPromise = (code
      ? fetch("/pair", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ code }) })
          .then((r) => r.json())
          .then((j) => {
            if (!j.ok) throw new Error(j.error);
            sessionStorage.setItem("swiftwin-token", j.token);
            return j.token;
          })
      : Promise.resolve(sessionStorage.getItem("swiftwin-token")).then(
          (t) =>
            t ||
            fetch("/token").then((r) => {
              if (!r.ok) throw new Error("This phone link has expired: ask Claude to show the phone again");
              return r.json().then((j) => j.token);
            }),
        )
    ).catch((e) => {
      if (!code) tokenPromise = null; // a spent pairing code stays spent: no falling back to /token
      throw e;
    });
  }
  return tokenPromise;
}

export async function call(cmd, args = {}, retried = false) {
  if (inTauri) {
    const { invoke } = await import("@tauri-apps/api/core");
    try {
      return await invoke("core", { cmd, args });
    } catch (e) {
      throw new Error(typeof e === "string" ? e : e?.message || String(e));
    }
  }
  const r = await fetch("/api/" + cmd, {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-SwiftWin-Token": await token() },
    body: JSON.stringify(args),
  });
  // SwiftWin restarted (new token) under a page that stayed open: get a fresh token once.
  const j = await r.json();
  if (r.status === 403 && /X-SwiftWin-Token/.test(j.error) && !retried) {
    sessionStorage.removeItem("swiftwin-token");
    tokenPromise = null;
    return call(cmd, args, true);
  }
  if (!j.ok) throw new Error(j.error);
  return j.result;
}
