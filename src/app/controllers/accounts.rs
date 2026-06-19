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
    if username.is_empty() {
        return;
    }
    let account = Account {
        uuid: Uuid::new_v4().simple().to_string(),
        username,
        account_type: AccountType::Offline,
        microsoft_auth: None,
    };
    state.config.lock().unwrap().add_account(account);
    refresh(state, weak);
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

pub fn start_microsoft(state: &AppState, weak: &Weak<MainWindow>) {
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
                    state.config.lock().unwrap().add_account(account);
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
