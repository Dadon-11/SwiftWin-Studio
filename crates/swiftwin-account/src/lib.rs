//! SwiftWin accounts and GitHub upload.
//!
//! * Sign-in is done by Supabase Auth (a managed service, so SwiftWin runs no server):
//!   email + password with 6-digit email codes, and Google / GitHub through the system
//!   browser with PKCE and a loopback redirect (RFC 8252).
//! * The refresh token lives in the Windows Credential Manager; access tokens stay in memory.
//!   Nothing secret is written to config files or logs (`Session`'s `Debug` is redacted).
//! * "Connect GitHub" uses GitHub's OAuth device flow (no client secret in the app) to get a
//!   `repo`-scoped token, also kept in the Credential Manager.
//! * `upload` makes a GitHub repository match a project folder through the Git Data API
//!   (no local git needed), creating a private repository the first time.
//!
//! Every network call has a fake in [`fake`] used by the tests and by `--mock`.

pub mod account;
pub mod fake;
pub mod github;
pub mod loopback;
pub mod pkce;
pub mod settings;
pub mod supabase;
pub mod upload;

pub use account::{Accounts, Opener};
pub use settings::AuthSettings;
pub use supabase::{Session, User};

/// Credential Manager names (target `swiftwin:<name>`).
pub const REFRESH_TOKEN: &str = "account-refresh-token";
pub const GITHUB_UPLOAD_TOKEN: &str = "github-upload-token";
