//! Lazy player-head avatars fetched from Crafatar.
//!
//! Network fetch + PNG decode happen on the tokio runtime; the decoded RGBA
//! pixels (which are `Send`) are cached in `AppState::avatar_cache`. Only the
//! UI thread turns them into a `slint::Image` (see [`image_for`]), so we never
//! move a non-`Send` Slint image across threads.

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

use crate::app::state::AppState;
use crate::app::ui;
use crate::MainWindow;

/// Per-account fetch state. `Pending` is recorded the moment a fetch is spawned
/// so we never fetch the same UUID twice.
#[derive(Clone)]
pub enum AvatarEntry {
    Pending,
    Ready { rgba: Vec<u8>, width: u32, height: u32 },
    Failed,
}

/// Full-body renders are cached in the same map under a `"body:"`-prefixed key,
/// so they never collide with the head avatars keyed by bare UUID.
fn body_key(uuid: &str) -> String {
    format!("body:{uuid}")
}

/// Build a `slint::Image` for a cached avatar, or an empty image when it is not
/// ready yet (the `Avatar` component falls back to its monogram in that case).
/// Must be called on the UI thread.
pub fn image_for(state: &AppState, uuid: &str) -> Image {
    image_from_cache(state, uuid)
}

/// Like [`image_for`] but for the larger full-body render (the Accounts hero).
/// Empty until the body fetch finishes; callers show the head/monogram meanwhile.
pub fn body_image_for(state: &AppState, uuid: &str) -> Image {
    if uuid.is_empty() {
        return Image::default();
    }
    image_from_cache(state, &body_key(uuid))
}

/// Shared cache lookup → `slint::Image`. Must run on the UI thread.
fn image_from_cache(state: &AppState, key: &str) -> Image {
    let cache = state.avatar_cache.lock().unwrap();
    if let Some(AvatarEntry::Ready { rgba, width, height }) = cache.get(key) {
        let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(*width, *height);
        buf.make_mut_bytes().copy_from_slice(rgba);
        Image::from_rgba8(buf)
    } else {
        Image::default()
    }
}

/// Ensure avatars for the given UUIDs are fetched. Spawns a background fetch for
/// each UUID not already in the cache; when one finishes it refreshes the
/// account list and active-account summary so the new head appears.
pub fn ensure(state: &AppState, weak: &slint::Weak<MainWindow>, uuids: Vec<String>) {
    let mut to_fetch = Vec::new();
    {
        let mut cache = state.avatar_cache.lock().unwrap();
        for uuid in uuids {
            if uuid.is_empty() || cache.contains_key(&uuid) {
                continue;
            }
            cache.insert(uuid.clone(), AvatarEntry::Pending);
            to_fetch.push(uuid);
        }
    }

    for uuid in to_fetch {
        let state = state.clone();
        let weak = weak.clone();
        state.rt.clone().spawn(async move {
            let entry = match fetch_head(&uuid).await {
                Some((rgba, width, height)) => AvatarEntry::Ready { rgba, width, height },
                None => AvatarEntry::Failed,
            };
            state.avatar_cache.lock().unwrap().insert(uuid, entry);

            let st = state.clone();
            let _ = weak.upgrade_in_event_loop(move |ui| {
                let config = st.config.lock().unwrap();
                ui::refresh_accounts(&ui, &config, &st);
                ui::refresh_summary(&ui, &config, &st);
            });
        });
    }
}

/// Ensure the full-body render for one account UUID is fetched. Used for the
/// active-account hero panel on the Accounts page; refreshes the summary when
/// the render arrives. Cheap no-op once cached.
pub fn ensure_body(state: &AppState, weak: &slint::Weak<MainWindow>, uuid: String) {
    if uuid.is_empty() {
        return;
    }
    let key = body_key(&uuid);
    {
        let mut cache = state.avatar_cache.lock().unwrap();
        if cache.contains_key(&key) {
            return;
        }
        cache.insert(key.clone(), AvatarEntry::Pending);
    }

    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        let entry = match fetch_body(&uuid).await {
            Some((rgba, width, height)) => AvatarEntry::Ready { rgba, width, height },
            None => AvatarEntry::Failed,
        };
        state.avatar_cache.lock().unwrap().insert(key, entry);

        let st = state.clone();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let config = st.config.lock().unwrap();
            ui::refresh_summary(&ui, &config, &st);
        });
    });
}

/// Fetch a 64px overlaid player head and decode it to RGBA8. Tries several
/// providers in order so a single service outage doesn't break avatars.
/// Returns `None` only when every provider fails (callers cache that as
/// `Failed`, falling back to the monogram avatar).
async fn fetch_head(uuid: &str) -> Option<(Vec<u8>, u32, u32)> {
    // `{}` is replaced with the account UUID. Ordered by preference.
    const PROVIDERS: [&str; 3] = [
        "https://mc-heads.net/avatar/{}/64",
        "https://minotar.net/helm/{}/64.png",
        "https://crafatar.com/avatars/{}?size=64&overlay",
    ];
    fetch_from(uuid, &PROVIDERS).await
}

/// Fetch a full-body player render (with armor/overlay) and decode it to RGBA8,
/// trying providers in order. Returns `None` only when every provider fails.
async fn fetch_body(uuid: &str) -> Option<(Vec<u8>, u32, u32)> {
    const PROVIDERS: [&str; 3] = [
        "https://mc-heads.net/body/{}/100",
        "https://minotar.net/armor/body/{}/100.png",
        "https://crafatar.com/renders/body/{}?size=100&overlay",
    ];
    fetch_from(uuid, &PROVIDERS).await
}

/// Try each provider template (with `{}` replaced by the UUID) until one decodes.
async fn fetch_from(uuid: &str, providers: &[&str]) -> Option<(Vec<u8>, u32, u32)> {
    for template in providers {
        let url = template.replace("{}", uuid);
        if let Some(decoded) = try_fetch(&url).await {
            return Some(decoded);
        }
    }
    None
}

async fn try_fetch(url: &str) -> Option<(Vec<u8>, u32, u32)> {
    let resp = reqwest::get(url).await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    let rgba = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some((rgba.into_raw(), width, height))
}
