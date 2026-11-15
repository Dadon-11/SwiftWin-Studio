//! End-to-end account and upload flows against the in-process fakes.

use std::sync::Arc;
use std::time::Duration;

use swiftwin_account::fake::{self, FakeCloud};
use swiftwin_account::upload::{self, Links, Options};
use swiftwin_account::{Accounts, REFRESH_TOKEN};
use swiftwin_signer::secrets::{MemoryStore, SecretStore};

fn tmp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("swiftwin-acct-{tag}-{}", rand::random::<u32>()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn accounts(cloud: &FakeCloud, store: Arc<MemoryStore>, dir: &std::path::Path) -> Accounts {
    Accounts::new(cloud.settings(), store, fake::fake_browser(), dir.to_path_buf())
}

#[test]
fn email_sign_up_confirm_sign_out_sign_in_restore() {
    let cloud = FakeCloud::start();
    let store = Arc::new(MemoryStore::default());
    let dir = tmp("email");
    let a = accounts(&cloud, store.clone(), &dir);
    assert_eq!(a.status()["signed_in"], false);
    assert!(a.sign_up("new@example.com", "short").is_err());
    let r = a.sign_up("New@Example.com", "a good password").unwrap();
    assert_eq!(r["code_sent"], true);
    assert!(a.sign_in("new@example.com", "a good password").unwrap_err().to_string().contains("Confirm your email"));
    assert!(a.confirm_sign_up("new@example.com", "000000").is_err());
    let u = a.confirm_sign_up("new@example.com", fake::SIGNUP_CODE).unwrap();
    assert_eq!((u.email.as_str(), u.provider.as_str(), u.plan.as_str()), ("new@example.com", "email", "free"));
    assert!(store.get(REFRESH_TOKEN).unwrap().is_some());
    // The profile file holds no token.
    let profile = std::fs::read_to_string(dir.join("account.json")).unwrap();
    assert!(profile.contains("new@example.com") && !profile.contains("refresh") && !profile.contains("at-"));

    // A fresh process restores the session from the stored refresh token.
    let b = accounts(&cloud, store.clone(), &dir);
    assert_eq!(b.restore().unwrap().unwrap().email, "new@example.com");
    assert_eq!(b.status()["signed_in"], true);
    assert!(b.access_token().unwrap().starts_with("at-"));

    b.sign_out().unwrap();
    assert!(store.get(REFRESH_TOKEN).unwrap().is_none());
    assert!(!dir.join("account.json").exists());
    assert_eq!(cloud.state.lock().unwrap().logouts, 1);
    assert!(b.sign_in("new@example.com", "wrong password").unwrap_err().to_string().contains("Wrong email or password"));
    b.sign_in("new@example.com", "a good password").unwrap();
    assert_eq!(b.status()["user"]["email"], "new@example.com");
}

#[test]
fn reused_refresh_token_signs_out_and_offline_keeps_profile() {
    let cloud = FakeCloud::start();
    let store = Arc::new(MemoryStore::default());
    let dir = tmp("reuse");
    let a = accounts(&cloud, store.clone(), &dir);
    a.sign_in(fake::DEMO_EMAIL, fake::DEMO_PASSWORD).unwrap();
    let old = store.get(REFRESH_TOKEN).unwrap().unwrap();
    a.restore().unwrap(); // rotates
    store.set(REFRESH_TOKEN, &old).unwrap(); // someone saved a stale token
    let b = accounts(&cloud, store.clone(), &dir);
    assert!(b.restore().unwrap().is_none());
    assert_eq!(b.status()["signed_in"], false);
    assert!(store.get(REFRESH_TOKEN).unwrap().is_none());

    // Offline: the service is unreachable, the cached profile keeps the person signed in.
    a.sign_in(fake::DEMO_EMAIL, fake::DEMO_PASSWORD).unwrap();
    let mut s = cloud.settings();
    s.supabase_url = "http://127.0.0.1:9".into();
    let c = Accounts::new(s, store.clone(), fake::fake_browser(), dir.clone());
    assert_eq!(c.restore().unwrap().unwrap().email, fake::DEMO_EMAIL);
    assert_eq!(c.status()["offline"], true);
}

#[test]
fn the_email_links_sign_in_on_this_pc() {
    let cloud = FakeCloud::start();
    let store = Arc::new(MemoryStore::default());
    let a = accounts(&cloud, store.clone(), &tmp("links"));
    a.sign_up("link@example.com", "a good password").unwrap();
    assert_eq!(a.status()["pending"], "email-signup");
    let link = cloud.email_link("link@example.com").unwrap();
    assert!(link.contains("redirect_to=http%3A%2F%2F127.0.0.1%3A"), "{link}");
    ureq::get(&link).call().unwrap(); // the person clicks the link: the browser lands on the app
    let u = a.wait_browser_sign_in(Duration::from_secs(10)).unwrap();
    assert_eq!(u.email, "link@example.com");
    assert_eq!(a.status()["recovered"], false);

    // Reset: the link signs in and asks for a new password.
    a.sign_out().unwrap();
    a.send_reset("link@example.com").unwrap();
    assert_eq!(a.status()["pending"], "email-reset");
    ureq::get(&cloud.email_link("link@example.com").unwrap()).call().unwrap();
    a.wait_browser_sign_in(Duration::from_secs(10)).unwrap();
    assert_eq!(a.status()["recovered"], true);
    assert!(a.set_new_password("short").is_err());
    a.set_new_password("another good one").unwrap();
    assert_eq!(a.status()["recovered"], false);
    assert_eq!(cloud.state.lock().unwrap().users["link@example.com"].password, "another good one");

    // A used link can't sign in again.
    a.sign_out().unwrap();
    a.send_reset("link@example.com").unwrap();
    let link = cloud.email_link("link@example.com").unwrap();
    ureq::get(&link).call().unwrap();
    a.wait_browser_sign_in(Duration::from_secs(10)).unwrap();
    a.sign_out().unwrap();
    a.send_reset("link@example.com").unwrap();
    let fresh = a.status();
    assert_eq!(fresh["pending"], "email-reset");
    let old = link.replace(&link.split("redirect_to=").nth(1).unwrap().to_string(), &cloud.email_link("link@example.com").unwrap().split("redirect_to=").nth(1).unwrap().to_string());
    let _ = ureq::get(&old).call();
    assert!(a.wait_browser_sign_in(Duration::from_secs(10)).is_err());
}

#[test]
fn password_reset_with_code() {
    let cloud = FakeCloud::start();
    let store = Arc::new(MemoryStore::default());
    let a = accounts(&cloud, store.clone(), &tmp("reset"));
    a.send_reset(fake::DEMO_EMAIL).unwrap();
    assert!(a.reset_password(fake::DEMO_EMAIL, "111111", "brand new pass").is_err());
    a.reset_password(fake::DEMO_EMAIL, fake::RESET_CODE, "brand new pass").unwrap();
    assert_eq!(cloud.state.lock().unwrap().users[fake::DEMO_EMAIL].password, "brand new pass");
    assert_eq!(a.status()["signed_in"], true);
}

#[test]
fn google_and_github_sign_in_through_the_browser_with_pkce() {
    let cloud = FakeCloud::start();
    for p in ["google", "github"] {
        let store = Arc::new(MemoryStore::default());
        let a = accounts(&cloud, store.clone(), &tmp(p));
        let url = a.start_browser_sign_in(p).unwrap();
        assert!(url.contains("code_challenge_method=s256") && url.contains("redirect_to=http%3A%2F%2F127.0.0.1%3A"));
        let u = a.wait_browser_sign_in(Duration::from_secs(10)).unwrap();
        assert_eq!(u.provider, p);
        assert_eq!(u.name.as_deref(), Some("Octo Cat"));
        assert!(store.get(REFRESH_TOKEN).unwrap().is_some());
    }
    let a = accounts(&cloud, Arc::new(MemoryStore::default()), &tmp("bad"));
    assert!(a.start_browser_sign_in("myspace").is_err());
}

#[test]
fn a_code_without_the_verifier_is_useless() {
    let cloud = FakeCloud::start();
    // A browser that never comes back: grab the code an attacker might see in the redirect.
    let a = Accounts::new(cloud.settings(), Arc::new(MemoryStore::default()), Arc::new(|_: &str| Ok(())), tmp("steal"));
    let url = a.start_browser_sign_in("github").unwrap();
    let redirect = ureq::AgentBuilder::new().redirects(0).build().get(&url).call().unwrap();
    let loc = redirect.header("location").unwrap().to_string();
    let code = loc.split("code=").nth(1).unwrap().to_string();
    let client = swiftwin_account::supabase::AuthClient::new(&cloud.url(), fake::ANON_KEY);
    assert!(client.exchange_code(&code, &swiftwin_account::pkce::verifier()).is_err());
    a.cancel_browser_sign_in();
}

#[test]
fn connect_github_and_upload_creates_private_repo_then_updates_it() {
    let cloud = FakeCloud::start();
    let store = Arc::new(MemoryStore::default());
    let dir = tmp("gh");
    let a = accounts(&cloud, store.clone(), &dir);
    a.sign_in(fake::DEMO_EMAIL, fake::DEMO_PASSWORD).unwrap();
    assert_eq!(a.github.status()["connected"], false);
    let c = a.github.connect().unwrap();
    assert_eq!(c["user_code"], "FAKE-CODE");
    assert_eq!(a.github.wait(Duration::from_secs(10)).unwrap(), "octo");
    assert_eq!(a.github.status()["login"], "octo");
    let token = a.github.token().unwrap();

    let proj = tmp("proj").join("Hello App");
    std::fs::create_dir_all(proj.join("Sources")).unwrap();
    std::fs::create_dir_all(proj.join("build")).unwrap();
    std::fs::write(proj.join("Sources/App.swift"), "import SwiftUI\n").unwrap();
    std::fs::write(proj.join("swiftwin.toml"), "name = \"Hello\"\n").unwrap();
    std::fs::write(proj.join("build/Hello"), "binary").unwrap();
    std::fs::write(proj.join("dev.p12"), "cert").unwrap();
    let links = Links::new(dir.join("uploads.json"));
    let api = cloud.settings().github_api;
    let steps = std::sync::Mutex::new(Vec::new());
    let progress = |m: &str, _d: usize, _t: usize| steps.lock().unwrap().push(m.to_string());

    let o = upload::upload(&api, &token, &proj, &Options::default(), &links, &progress).unwrap();
    assert_eq!(o.repo, "octo/Hello-App");
    assert!(o.created && o.commit.is_some());
    assert_eq!(o.uploaded, 2);
    assert!(o.skipped.iter().any(|s| s.path == "dev.p12"));
    assert!(cloud.state.lock().unwrap().repos["octo/Hello-App"].private);
    let files = cloud.repo_files("octo/Hello-App");
    let mut names: Vec<_> = files.keys().cloned().collect();
    names.sort();
    assert_eq!(names, vec!["Sources/App.swift", "swiftwin.toml"]);
    assert!(steps.lock().unwrap().iter().any(|s| s.starts_with("Creating the repository")));

    // Nothing changed: no commit, no blob uploads.
    let posts = cloud.state.lock().unwrap().blob_posts;
    let o = upload::upload(&api, &token, &proj, &Options::default(), &links, &progress).unwrap();
    assert!(o.commit.is_none() && !o.created && o.uploaded == 0);
    assert_eq!(cloud.state.lock().unwrap().blob_posts, posts);

    // One file changed, one deleted: only the changed one is sent.
    std::fs::write(proj.join("Sources/App.swift"), "import SwiftUI // v2\n").unwrap();
    std::fs::remove_file(proj.join("swiftwin.toml")).unwrap();
    let o = upload::upload(&api, &token, &proj, &Options { message: Some("v2".into()), ..Default::default() }, &links, &progress).unwrap();
    assert_eq!(o.uploaded, 1);
    assert_eq!(cloud.state.lock().unwrap().repos["octo/Hello-App"].commits, 3);
    assert_eq!(cloud.repo_files("octo/Hello-App").keys().collect::<Vec<_>>(), vec!["Sources/App.swift"]);

    // A second folder with the same name can't silently take over the repo.
    let other = tmp("proj2").join("Hello App");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("a.swift"), "x").unwrap();
    let e = upload::upload(&api, &token, &other, &Options::default(), &links, &progress).unwrap_err();
    assert!(e.to_string().contains("already have a repository named Hello-App"), "{e}");

    // A private key inside an ordinary file stops the upload.
    std::fs::write(proj.join("Sources/Keys.swift"), "let k = \"-----BEGIN PRIVATE KEY-----\"").unwrap();
    assert!(upload::upload(&api, &token, &proj, &Options::default(), &links, &progress).unwrap_err().to_string().contains("private key"));

    // Signing out disconnects GitHub too.
    a.sign_out().unwrap();
    assert!(a.github.token().is_err());
    assert_eq!(a.github.status()["connected"], false);
}

#[test]
fn github_denied_and_unconfigured() {
    let cloud = FakeCloud::start();
    cloud.state.lock().unwrap().device_deny = true;
    let a = accounts(&cloud, Arc::new(MemoryStore::default()), &tmp("deny"));
    a.github.connect().unwrap();
    assert!(a.github.wait(Duration::from_secs(10)).unwrap_err().to_string().contains("cancelled"));
    let none = Accounts::new(Default::default(), Arc::new(MemoryStore::default()), fake::fake_browser(), tmp("none"));
    assert!(!none.configured());
    assert!(none.sign_in("a@b.co", "password1").is_err());
    assert!(none.restore().unwrap().is_none());
    assert!(none.github.connect().is_err());
}

#[test]
fn profile_name_password_and_picture() {
    let cloud = FakeCloud::start();
    let store = Arc::new(MemoryStore::default());
    let dir = tmp("profile");
    let a = accounts(&cloud, store.clone(), &dir);
    assert!(a.update_name("Ada").is_err(), "signed out: nothing to rename");
    let u = a.sign_in(fake::DEMO_EMAIL, fake::DEMO_PASSWORD).unwrap();
    assert!(u.email_confirmed && u.created_at.as_deref() == Some("2026-11-01T09:30:00Z") && u.avatar_url.is_none());
    assert!(a.avatar().is_none(), "email accounts have no picture: the UI shows initials");

    // Name: checked, saved at the service, in memory and in account.json.
    assert!(a.update_name("   ").is_err() && a.update_name(&"x".repeat(65)).is_err() && a.update_name("a\nb").is_err());
    assert_eq!(a.update_name("  Ada Lovelace ").unwrap().name.as_deref(), Some("Ada Lovelace"));
    assert_eq!(a.status()["user"]["name"], "Ada Lovelace");
    assert_eq!(cloud.state.lock().unwrap().users[fake::DEMO_EMAIL].name.as_deref(), Some("Ada Lovelace"));
    assert!(std::fs::read_to_string(dir.join("account.json")).unwrap().contains("Ada Lovelace"));

    // Password: the current one is checked first; the new one must be valid and different.
    assert!(a.change_password("wrong one", "brand new pass").unwrap_err().to_string().contains("current password is wrong"));
    assert!(a.change_password(fake::DEMO_PASSWORD, "short").is_err());
    assert!(a.change_password(fake::DEMO_PASSWORD, fake::DEMO_PASSWORD).is_err());
    a.change_password(fake::DEMO_PASSWORD, "brand new pass").unwrap();
    assert_eq!(cloud.state.lock().unwrap().users[fake::DEMO_EMAIL].password, "brand new pass");
    assert_eq!(a.status()["signed_in"], true, "still signed in after the change");
    assert!(store.get(REFRESH_TOKEN).unwrap().is_some());

    // Google / GitHub accounts: no password here; the picture comes as a data: URL.
    let g = accounts(&cloud, Arc::new(MemoryStore::default()), &tmp("profile-g"));
    g.start_browser_sign_in("google").unwrap();
    let gu = g.wait_browser_sign_in(Duration::from_secs(10)).unwrap();
    assert!(gu.avatar_url.as_deref().unwrap().ends_with("/avatar.svg"));
    assert!(g.change_password("x", "brand new pass").unwrap_err().to_string().contains("Google"));
    let pic = g.avatar().unwrap();
    assert!(pic.starts_with("data:image/svg+xml;base64,"), "{pic}");
    // A name set in SwiftWin wins over the one Google sends.
    assert_eq!(g.update_name("Grace").unwrap().name.as_deref(), Some("Grace"));
}

#[test]
fn delete_my_account() {
    let cloud = FakeCloud::start();
    let store = Arc::new(MemoryStore::default());
    let dir = tmp("delete");
    let a = accounts(&cloud, store.clone(), &dir);
    assert!(a.delete_account(fake::DEMO_EMAIL).is_err(), "signed out: nothing to delete");
    a.sign_in(fake::DEMO_EMAIL, fake::DEMO_PASSWORD).unwrap();
    store.set(swiftwin_account::GITHUB_UPLOAD_TOKEN, fake::GH_TOKEN).unwrap();

    // The typed confirmation must be the account's email; nothing happens otherwise.
    assert!(a.delete_account("delete").unwrap_err().to_string().contains(fake::DEMO_EMAIL));
    assert!(cloud.state.lock().unwrap().users.contains_key(fake::DEMO_EMAIL));

    // The SQL function isn't installed yet: a clear message, still signed in.
    cloud.state.lock().unwrap().no_delete_fn = true;
    assert!(a.delete_account(fake::DEMO_EMAIL).unwrap_err().to_string().contains("delete_my_account"));
    assert_eq!(a.status()["signed_in"], true);
    cloud.state.lock().unwrap().no_delete_fn = false;

    // Deleted at the service, and everything is forgotten on this PC.
    a.delete_account(&format!("  {}  ", fake::DEMO_EMAIL.to_uppercase())).unwrap();
    assert!(!cloud.state.lock().unwrap().users.contains_key(fake::DEMO_EMAIL));
    assert_eq!(a.status()["signed_in"], false);
    assert!(store.get(REFRESH_TOKEN).unwrap().is_none());
    assert!(store.get(swiftwin_account::GITHUB_UPLOAD_TOKEN).unwrap().is_none());
    assert!(!dir.join("account.json").exists());
    assert!(a.sign_in(fake::DEMO_EMAIL, fake::DEMO_PASSWORD).is_err(), "the account is gone");
}

#[test]
fn email_sign_in_follows_the_service_setting() {
    let cloud = FakeCloud::start();
    let a = accounts(&cloud, Arc::new(MemoryStore::default()), &tmp("email-on"));
    a.restore().unwrap();
    assert_eq!(a.status()["email"], true);
    cloud.state.lock().unwrap().email_off = true;
    let b = accounts(&cloud, Arc::new(MemoryStore::default()), &tmp("email-off"));
    b.restore().unwrap();
    assert_eq!(b.status()["email"], false, "the app hides email sign-in when the owner turned it off");
}
