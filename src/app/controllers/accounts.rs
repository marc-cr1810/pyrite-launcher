//! Account management: offline accounts and Microsoft device-code login.

use std::sync::atomic::Ordering;
use std::time::Duration;

use slint::Weak;
use uuid::Uuid;

use crate::app::state::AppState;
use crate::app::ui;
use crate::core::api::ApiClient;
use crate::core::config::{Account, AccountType, MicrosoftAuth};
use crate::MainWindow;

pub fn add_offline(state: &AppState, weak: &Weak<MainWindow>, username: String) {
    let username = username.trim().to_string();

    // Validate against Minecraft's offline username rules so the launcher fails
    // loudly (inline) rather than silently accepting names the game rejects.
    if let Err(msg) = validate_offline_username(state, &username) {
        set_account_error(weak, msg);
        return;
    }

    let account = Account {
        uuid: Uuid::new_v4().simple().to_string(),
        username,
        account_type: AccountType::Offline,
        microsoft_auth: None,
    };
    state.config.lock().unwrap().add_account(account);
    set_account_error(weak, "");
    refresh(state, weak);
}

/// Check an offline username: 3–16 chars of `[A-Za-z0-9_]`, not already added.
fn validate_offline_username(state: &AppState, username: &str) -> Result<(), &'static str> {
    if username.is_empty() {
        return Err("Enter a username.");
    }
    let len = username.chars().count();
    if !(3..=16).contains(&len) {
        return Err("Usernames must be 3–16 characters.");
    }
    if !username.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("Use only letters, numbers and underscores.");
    }
    let exists = state
        .config
        .lock()
        .unwrap()
        .accounts
        .iter()
        .any(|a| a.username.eq_ignore_ascii_case(username));
    if exists {
        return Err("That username is already added.");
    }
    Ok(())
}

/// Set (or clear) the Accounts-page inline error from any thread.
fn set_account_error(weak: &Weak<MainWindow>, msg: impl Into<String>) {
    let msg = msg.into();
    let _ = weak.upgrade_in_event_loop(move |ui| ui::set_account_error(&ui, msg));
}

pub fn select(state: &AppState, weak: &Weak<MainWindow>, uuid: String) {
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.active_account_uuid = Some(uuid);
        let _ = cfg.save();
    }
    refresh(state, weak);
}

pub fn remove(state: &AppState, weak: &Weak<MainWindow>, uuid: String) {
    state.config.lock().unwrap().remove_account(&uuid);
    refresh(state, weak);
}

pub fn cancel_microsoft(state: &AppState) {
    state.ms_cancel.store(true, Ordering::SeqCst);
}

/// Open a URL in the user's default browser.
pub fn open_url(url: String) {
    let url = url.trim().to_string();
    if !url.is_empty() {
        let _ = open::that_detached(url);
    }
}

/// Copy text to the system clipboard.
pub fn copy_code(text: String) {
    if text.trim().is_empty() {
        return;
    }
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        let _ = clipboard.set_text(text);
    }
}

/// Begin a fresh Microsoft device-code login.
pub fn start_microsoft(state: &AppState, weak: &Weak<MainWindow>) {
    run_microsoft(state, weak, None);
}

/// Re-run the Microsoft login for an existing (expired) account. The new tokens
/// replace the account by UUID; if the user signs into a *different* account,
/// the stale one is dropped.
pub fn relogin_account(state: &AppState, weak: &Weak<MainWindow>, uuid: String) {
    run_microsoft(state, weak, Some(uuid));
}

fn run_microsoft(state: &AppState, weak: &Weak<MainWindow>, expected_uuid: Option<String>) {
    state.ms_cancel.store(false, Ordering::SeqCst);
    let state = state.clone();
    let weak = weak.clone();

    state.rt.clone().spawn(async move {
        let api = ApiClient::new();

        let dev = match api.request_device_code().await {
            Ok(d) => d,
            Err(e) => {
                ui_ms(&weak, true, false, "", "", format!("Failed to start login: {e}"));
                return;
            }
        };

        ui_ms(
            &weak,
            true,
            true,
            dev.user_code.clone(),
            dev.verification_uri.clone(),
            "Waiting for you to authorize in the browser…",
        );

        let interval = Duration::from_secs(dev.interval.max(1));
        let mut remaining = dev.expires_in;

        loop {
            if state.ms_cancel.load(Ordering::SeqCst) {
                ui_ms(&weak, false, false, "", "", "");
                return;
            }
            if remaining == 0 {
                ui_ms(&weak, true, false, "", "", "Login timed out. Please try again.");
                return;
            }
            tokio::time::sleep(interval).await;
            remaining = remaining.saturating_sub(interval.as_secs());

            match api.poll_token(&dev.device_code).await {
                Ok(Some(token)) => {
                    let mc = match api.login_with_microsoft(&token.access_token).await {
                        Ok(r) => r,
                        Err(e) => {
                            ui_ms(&weak, true, false, "", "", format!("Login failed: {e}"));
                            return;
                        }
                    };
                    let profile = match api.fetch_profile(&mc.access_token).await {
                        Ok(p) => p,
                        Err(e) => {
                            ui_ms(&weak, true, false, "", "", format!("Could not fetch profile: {e}"));
                            return;
                        }
                    };
                    let account = Account {
                        uuid: profile.id,
                        username: profile.name,
                        account_type: AccountType::Microsoft,
                        microsoft_auth: Some(MicrosoftAuth {
                            access_token: mc.access_token,
                            refresh_token: token.refresh_token,
                            expires_at: Some(
                                chrono::Utc::now() + chrono::Duration::seconds(mc.expires_in as i64),
                            ),
                        }),
                    };
                    {
                        let mut cfg = state.config.lock().unwrap();
                        // Re-login into a different account: discard the stale one.
                        if let Some(old) =
                            expected_uuid.as_deref().filter(|old| *old != account.uuid)
                        {
                            cfg.remove_account(old);
                        }
                        cfg.add_account(account);
                    }
                    refresh(&state, &weak);
                    ui_ms(&weak, false, false, "", "", "");
                    return;
                }
                Ok(None) => {} // still pending
                Err(e) => {
                    ui_ms(&weak, true, false, "", "", format!("Authentication failed: {e}"));
                    return;
                }
            }
        }
    });
}

/// Refresh a Microsoft account's token now, regardless of expiry, and report the
/// outcome on the Accounts page. Offline accounts are a no-op. Triggered by the
/// per-account "refresh" button.
pub fn refresh_account(state: &AppState, weak: &Weak<MainWindow>, uuid: String) {
    let account = match state
        .config
        .lock()
        .unwrap()
        .accounts
        .iter()
        .find(|a| a.uuid == uuid)
        .cloned()
    {
        Some(a) => a,
        None => return,
    };
    if account.account_type != AccountType::Microsoft {
        return;
    }

    let state = state.clone();
    let weak = weak.clone();
    set_account_error(&weak, "Refreshing session…");
    state.rt.clone().spawn(async move {
        match refresh_if_needed(&state, &account, true).await {
            Ok(_) => {
                refresh(&state, &weak);
                set_account_error(&weak, "");
            }
            Err(e) => set_account_error(&weak, format!("Refresh failed: {e}. Try re-login.")),
        }
    });
}

/// Ensure a Microsoft account's token is valid, refreshing it if it has expired
/// (or is within 60s of doing so), or unconditionally when `force` is set.
/// Returns the (possibly updated) account and persists any refresh to config.
/// Offline accounts are returned unchanged.
pub async fn refresh_if_needed(
    state: &AppState,
    account: &Account,
    force: bool,
) -> Result<Account, String> {
    let ms = match (&account.account_type, &account.microsoft_auth) {
        (AccountType::Microsoft, Some(ms)) => ms,
        _ => return Ok(account.clone()),
    };

    let needs_refresh = force
        || match ms.expires_at {
            Some(exp) => exp <= chrono::Utc::now() + chrono::Duration::seconds(60),
            None => true,
        };
    if !needs_refresh {
        return Ok(account.clone());
    }

    let api = ApiClient::new();
    let token = api.refresh_token(&ms.refresh_token).await?;
    let mc = api.login_with_microsoft(&token.access_token).await?;

    let updated = Account {
        uuid: account.uuid.clone(),
        username: account.username.clone(),
        account_type: AccountType::Microsoft,
        microsoft_auth: Some(MicrosoftAuth {
            access_token: mc.access_token,
            refresh_token: token.refresh_token,
            expires_at: Some(chrono::Utc::now() + chrono::Duration::seconds(mc.expires_in as i64)),
        }),
    };

    {
        let mut cfg = state.config.lock().unwrap();
        if let Some(acc) = cfg.accounts.iter_mut().find(|a| a.uuid == updated.uuid) {
            *acc = updated.clone();
        }
        let _ = cfg.save();
    }
    Ok(updated)
}

fn refresh(state: &AppState, weak: &Weak<MainWindow>) {
    let state = state.clone();
    let _ = weak.upgrade_in_event_loop(move |ui| ui::refresh_all(&ui, &state));
}

fn ui_ms(
    weak: &Weak<MainWindow>,
    open: bool,
    active: bool,
    code: impl Into<String>,
    uri: impl Into<String>,
    message: impl Into<String>,
) {
    let (code, uri, message) = (code.into(), uri.into(), message.into());
    let _ = weak.upgrade_in_event_loop(move |ui| {
        ui::set_ms_dialog(&ui, open, active, code, uri, message);
    });
}
