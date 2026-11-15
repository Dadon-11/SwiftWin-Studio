//! SwiftWin desktop app: a thin Tauri shell around `swiftwin_app_core::AppCore`.
//! Run with `--mock` (or SWIFTWIN_MOCK=1) to use the simulated iPhone.
//!
//! On start it also serves AppCore on 127.0.0.1 (random port, per-run token)
//! and publishes that in `app-bridge.json`, so `swiftwin mcp` (Claude Code)
//! drives the phone *through the app* and AI mode can show every action.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use swiftwin_app_core::{bridge, http, AppCore};
use tauri::Manager;

/// The only command: forwards to `AppCore::dispatch` on a worker thread. The two `app_update_*`
/// commands are handled here (Tauri's updater); the MCP bridge goes to AppCore and can't reach them.
#[tauri::command]
async fn core(app: tauri::AppHandle, state: tauri::State<'_, Arc<AppCore>>, cmd: String, args: serde_json::Value) -> Result<serde_json::Value, String> {
    match cmd.as_str() {
        "app_update_check" => return update_check(&app).await.map_err(|e| e.to_string()),
        "app_update_install" => return update_install(&app).await.map_err(|e| e.to_string()),
        "setup_sdk_signin" if !state.force_mock => return apple_signin(&app, state.inner().clone()).map_err(|e| e.to_string()),
        _ => {}
    }
    let c = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || c.dispatch(&cmd, &args).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

/// Apple's own sign-in page, in a private window (incognito: no cookies on disk) that may only show
/// apple.com pages and has no access to SwiftWin's commands. Signed in, Apple starts the Xcode
/// download; SwiftWin cancels it in the window and downloads it itself (resumable) with the session
/// cookies, which stay in memory. SwiftWin never sees the password.
// UNTESTED-ON-DEVICE: Apple's real sign-in and download in WebView2 (needs an Apple ID).
fn apple_signin(app: &tauri::AppHandle, core: Arc<AppCore>) -> tauri::Result<serde_json::Value> {
    if let Some(w) = app.get_webview_window("apple") {
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(serde_json::json!({"open": true}));
    }
    let url: tauri::Url = swiftwin_app_core::setup::signin_url().parse().expect("valid sign-in URL");
    let taken = Arc::new(AtomicBool::new(false));
    tauri::WebviewWindowBuilder::new(app, "apple", tauri::WebviewUrl::External(url))
        .title("Sign in with your Apple ID · SwiftWin")
        .inner_size(560.0, 780.0)
        .incognito(true)
        .on_navigation(|u| u.scheme() == "https" && u.host_str().is_some_and(|h| h == "apple.com" || h.ends_with(".apple.com")))
        .on_download(move |webview, event| {
            if let tauri::webview::DownloadEvent::Requested { url, .. } = event {
                let wanted = url.to_string();
                if swiftwin_app_core::setup::is_wanted_download(&wanted) && !taken.swap(true, Ordering::SeqCst) {
                    let core = core.clone();
                    // Reading cookies inside this handler deadlocks on Windows (wry#583): another thread.
                    std::thread::spawn(move || {
                        let cookies = webview.cookies_for_url(url).unwrap_or_default();
                        let header = cookies.iter().map(|c| format!("{}={}", c.name(), c.value())).collect::<Vec<_>>().join("; ");
                        core.setup.start_sdk_download(wanted, header);
                        let _ = webview.window().close();
                    });
                }
            }
            false // never let the window save files
        })
        .build()?;
    Ok(serde_json::json!({"open": true}))
}

/// The update found by the last check, kept for `app_update_install`.
static PENDING_UPDATE: std::sync::Mutex<Option<tauri_plugin_updater::Update>> = std::sync::Mutex::new(None);

/// The `latest.json` URL: `SWIFTWIN_UPDATE_URL`, else `update_url` in config.json, else
/// `plugins.updater.endpoints` in tauri.conf.json (empty until there is a public release feed). Updates are verified against the public key in tauri.conf.json.
fn update_url() -> Option<String> {
    std::env::var("SWIFTWIN_UPDATE_URL").ok().or_else(|| swiftwin_core::config::Config::load().update_url).filter(|u| !u.trim().is_empty())
}

async fn update_check(app: &tauri::AppHandle) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    use tauri_plugin_updater::UpdaterExt;
    use tauri_plugin_updater::Error::EmptyEndpoints;
    let mut b = app.updater_builder();
    if let Some(url) = update_url() {
        b = b.endpoints(vec![url.parse()?])?;
    }
    // No override and no `endpoints` in tauri.conf.json: updates are off.
    let updater = match b.build() {
        Err(EmptyEndpoints) => return Ok(serde_json::json!({"configured": false})),
        r => r?,
    };
    let update = match updater.check().await {
        Err(EmptyEndpoints) => return Ok(serde_json::json!({"configured": false})),
        r => r?,
    };
    let v = match &update {
        Some(u) => serde_json::json!({"configured": true, "available": true, "version": u.version, "notes": u.body}),
        None => serde_json::json!({"configured": true, "available": false}),
    };
    *PENDING_UPDATE.lock().unwrap() = update;
    Ok(v)
}

/// Download, check the signature, run the installer (passive), then restart.
async fn update_install(app: &tauri::AppHandle) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let update = PENDING_UPDATE.lock().unwrap().take().ok_or("no update to install: check for updates first")?;
    update.download_and_install(|_, _| {}, || {}).await?;
    bridge::remove();
    app.restart();
}

/// Show (or create) the main window or the phone-only window (`--phone`: just the iPhone mirror, for
/// working in the Claude Code app or a terminal; Claude opens it with the `show_phone` MCP tool).
fn show_window(app: &tauri::AppHandle, phone: bool) {
    let label = if phone { "phone" } else { "main" };
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let built = if phone {
        tauri::WebviewWindowBuilder::new(app, "phone", tauri::WebviewUrl::App("index.html".into()))
            .initialization_script("window.__SWIFTWIN_VIEW = 'phone';")
            .title("iPhone · SwiftWin")
            .inner_size(430.0, 900.0)
            .min_inner_size(300.0, 520.0)
            .build()
    } else {
        match app.config().app.windows.iter().find(|w| w.label == "main") {
            Some(c) => tauri::WebviewWindowBuilder::from_config(app, c).and_then(|b| b.build()),
            None => return,
        }
    };
    if let Err(e) = built {
        eprintln!("swiftwin: could not open the {label} window: {e}");
    }
}

/// Set once the app serves Claude's phone pane (`--pane`): it then lives in the system tray and keeps
/// running with no window open, so closing SwiftWin Studio doesn't cut the phone off in Claude.
static BACKGROUND: AtomicBool = AtomicBool::new(false);

/// Background mode: a tray icon (Open SwiftWin Studio, Quit) and no window.
fn enter_background(app: &tauri::AppHandle) {
    if BACKGROUND.swap(true, Ordering::SeqCst) {
        return;
    }
    use tauri::menu::{Menu, MenuItem};
    let built = (|| -> tauri::Result<()> {
        let open = MenuItem::with_id(app, "open", "Open SwiftWin Studio", true, None::<&str>)?;
        let quit = MenuItem::with_id(app, "quit", "Quit SwiftWin", true, None::<&str>)?;
        let mut tray = tauri::tray::TrayIconBuilder::with_id("swiftwin")
            .tooltip("SwiftWin: the iPhone for Claude")
            .menu(&Menu::with_items(app, &[&open, &quit])?)
            .on_menu_event(|app, e| match e.id.as_ref() {
                "open" => show_window(app, false),
                "quit" => app.exit(0),
                _ => {}
            });
        if let Some(icon) = app.default_window_icon() {
            tray = tray.icon(icon.clone());
        }
        tray.build(app)?;
        Ok(())
    })();
    if let Err(e) = built {
        eprintln!("swiftwin: could not add the tray icon: {e}");
    }
}

fn main() {
    let mock = std::env::args().any(|a| a == "--mock");
    let phone = std::env::args().any(|a| a == "--phone");
    // `--pane`: started for the Claude app's Browser pane (MCP `show_phone`), which shows the phone
    // itself: no window, just the phone bridge and a tray icon.
    let pane = std::env::args().any(|a| a == "--pane");
    let app_core = Arc::new(AppCore::new(mock));
    let bridge_core = app_core.clone();
    tauri::Builder::default()
        // One app per user: a second launch (say, a double click on the shortcut while the app is
        // open) brings the open window forward and exits before `setup`, so it never publishes a
        // bridge of its own. On the owner's PC a second instance overwrote `app-bridge.json`; once
        // it closed, `swiftwin mcp` no longer reached the open app and AI mode showed nothing.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.iter().any(|a| a == "--pane") {
                enter_background(app);
            } else {
                show_window(app, args.iter().any(|a| a == "--phone"));
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(move |app| {
            if pane {
                enter_background(app.handle());
            } else {
                show_window(app.handle(), phone);
            }
            // The phone page's Account / Signing / Studio buttons: bring the Studio window forward on that page.
            let handle = app.handle().clone();
            let _ = bridge_core.studio_opener.set(Box::new(move |page: &str| {
                let (h, page) = (handle.clone(), page.to_string());
                let _ = handle.run_on_main_thread(move || {
                    show_window(&h, false);
                    // shortcut: a Studio window created just now may miss the page (it isn't loaded yet); it still opens.
                    if let Some(w) = h.get_webview_window("main") {
                        let _ = w.eval(format!("window.__swiftwinShow && window.__swiftwinShow({page:?})"));
                    }
                });
            }));
            // The bridge also serves the app's own pages, so the Claude app's Browser pane can show the phone.
            let assets = app.asset_resolver();
            let ui = http::Ui::Embedded(Arc::new(move |p: &str| assets.get(p.to_string()).map(|a| a.bytes)));
            match http::serve(bridge_core.clone(), 0, ui, std::time::Duration::ZERO) {
                Ok(port) => {
                    if let Err(e) = bridge::write(port, bridge_core.token()) {
                        eprintln!("swiftwin: could not publish the MCP bridge: {e}");
                    }
                }
                Err(e) => eprintln!("swiftwin: could not start the MCP bridge: {e}"),
            }
            Ok(())
        })
        .manage(app_core)
        .invoke_handler(tauri::generate_handler![core])
        .build(tauri::generate_context!())
        .expect("error while building SwiftWin")
        .run(|_app, event| match event {
            // The last window closed (code None): stay in the tray while Claude's phone pane needs the bridge.
            tauri::RunEvent::ExitRequested { code: None, api, .. } if BACKGROUND.load(Ordering::SeqCst) => api.prevent_exit(),
            tauri::RunEvent::Exit => bridge::remove(),
            _ => {}
        });
}
