//! Account and session management on top of InnerTube: browser import,
//! channel selection, validation, persistence in the OS credential store
//! (D9) and keep-alive.
//!
//! A session is saved right after import with no channel
//! (state `ChoosingChannel`); it becomes `SignedIn` only after the user picks
//! a channel and a private surface loads as that channel (D7, Meiro #1).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use encore_auth::import::{self, ImportError};
use encore_auth::store::{SecretStore, load_session, save_session};
use encore_auth::{ChannelSelection, Session};
use encore_core::error::{AppError, ErrorCode, Result};
use encore_core::model::{AuthState, Channel};
use parking_lot::Mutex;

use crate::{InnerTube, YtMusic};

pub struct Accounts {
    pub ytm: Arc<YtMusic>,
    store: Arc<dyn SecretStore>,
    channels: Mutex<Vec<(Channel, ChannelSelection)>>,
    expired: Mutex<Option<String>>,
    /// When the cookies were last rotated or re-read from the browser (D38).
    refreshed: Mutex<Option<Instant>>,
    refreshing: tokio::sync::Mutex<()>,
    /// A scheduled re-read couldn't get the browser's key without asking:
    /// only a session that stopped working asks again (D38).
    follow_paused: AtomicBool,
    /// The user didn't let a forced re-read have the key either: nothing
    /// asks again until they import or sign in (D38).
    follow_denied: AtomicBool,
}

/// How a session's cookies are kept current (D38).
#[derive(Clone, Debug, PartialEq)]
enum Upkeep {
    /// An in-app sign-in: Encore alone holds it, so it rotates it as a
    /// browser would.
    Rotate,
    /// Imported from this browser profile, which keeps rotating the same
    /// session: Encore follows it by reading its cookies again.
    Follow(String),
}

fn upkeep(source: &str) -> Option<Upkeep> {
    if source == "webview" {
        return Some(Upkeep::Rotate);
    }
    let (browser, profile) = source.split_once(':')?;
    (import::Browser::from_key(browser).is_some() && !profile.is_empty()).then(|| Upkeep::Follow(source.to_string()))
}

/// Rotate an in-app sign-in this often, as a browser tab does.
const ROTATE_EVERY: Duration = Duration::from_secs(10 * 60);
/// Read an imported session from its browser again this often.
const FOLLOW_EVERY: Duration = Duration::from_secs(15 * 60);
/// A forced refresh (something was refused) at most this often.
const FORCED_GAP: Duration = Duration::from_secs(60);

/// What checking a refused session found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// Refreshed, and a private page loads again.
    Working,
    /// YouTube Music says it is signed out.
    Expired,
    /// Couldn't tell (no connection, a bot check, no session).
    Unknown,
}

fn store_err(e: encore_auth::store::StoreError) -> AppError {
    AppError::new(ErrorCode::Keychain, e.to_string())
}

impl Accounts {
    pub fn new(ytm: Arc<YtMusic>, store: Arc<dyn SecretStore>) -> Arc<Self> {
        let a = Arc::new(Self { ytm, store, channels: Mutex::new(vec![]), expired: Mutex::new(None), refreshed: Mutex::new(None), refreshing: tokio::sync::Mutex::new(()), follow_paused: AtomicBool::new(false), follow_denied: AtomicBool::new(false) });
        // Persist rotated cookies (only for sessions that have a channel or are pending).
        let w = Arc::downgrade(&a);
        a.it().on_session_change(move |s| {
            if let Some(a) = w.upgrade() {
                let _ = save_session(a.store.as_ref(), s);
            }
        });
        a
    }

    pub fn it(&self) -> &Arc<InnerTube> {
        &self.ytm.it
    }

    /// Load the stored session at startup (one read of the credential store).
    pub fn restore(&self) -> Result<AuthState> {
        match load_session(self.store.as_ref()).map_err(store_err)? {
            Some(s) if s.jar.has_login() => {
                self.it().set_session(Some(s));
                Ok(self.state())
            }
            _ => Ok(AuthState::Anonymous),
        }
    }

    pub fn state(&self) -> AuthState {
        let Some(s) = self.it().session() else { return AuthState::Anonymous };
        if let Some(reason) = self.expired.lock().clone() {
            return AuthState::Expired { source: s.source.clone(), reason };
        }
        let chans: Vec<Channel> = self.channels.lock().iter().map(|c| c.0.clone()).collect();
        match &s.channel {
            None => AuthState::ChoosingChannel { channels: chans, source: s.source.clone() },
            Some(sel) => {
                let key = format!("{}:{}", sel.auth_user, sel.page_id.as_deref().unwrap_or("primary"));
                let ch = chans.iter().find(|c| c.key == key).cloned().unwrap_or(Channel {
                    key,
                    name: sel.name.clone(),
                    handle: sel.handle.clone(),
                    page_id: sel.page_id.clone(),
                    auth_user: sel.auth_user,
                    is_brand: sel.page_id.is_some(),
                    photo: None,
                });
                AuthState::SignedIn { channel: ch, channels: chans, source: s.source.clone(), premium: None }
            }
        }
    }

    pub fn mark_expired(&self, reason: &str) {
        *self.expired.lock() = Some(reason.to_string());
    }

    /// Import cookies from a browser profile, rotate them immediately and
    /// list the channels. The session is stored as "choosing a channel".
    pub async fn import_browser(&self, profile_id: &str, keychain_timeout: Duration) -> Result<AuthState> {
        let pid = profile_id.to_string();
        let jar = tokio::task::spawn_blocking(move || import::import(&pid, keychain_timeout)).await.map_err(|e| AppError::internal(e.to_string()))?.map_err(|e| match e {
            ImportError::KeychainDenied(_) | ImportError::KeyUnavailable(_) | ImportError::BrowserRunning(_) => AppError::new(ErrorCode::Keychain, e.to_string()).with_action("retryImport"),
            // Nothing to retry: the in-app sign-in is the way in.
            ImportError::AppBound(_) | ImportError::DpapiFailed(_) => AppError::new(ErrorCode::Rejected, e.to_string()).with_action("signIn"),
            ImportError::NoLogin => AppError::new(ErrorCode::AuthRequired, e.to_string()),
            other => AppError::new(ErrorCode::Rejected, other.to_string()),
        })?;
        self.adopt(Session::new(jar, profile_id)).await
    }

    /// Take a fresh cookie jar (import or in-app sign-in) and move to the
    /// channel picker.
    pub async fn adopt(&self, session: Session) -> Result<AuthState> {
        let previous = self.it().session();
        let was_expired = self.expired.lock().take();
        *self.refreshed.lock() = Some(Instant::now());
        self.follow_paused.store(false, Ordering::Relaxed);
        self.follow_denied.store(false, Ordering::Relaxed);
        self.it().set_session(Some(session));
        // D8: sync rotated cookies right away, before the first InnerTube call.
        // (Rotated cookies are saved as they arrive, see `new`.)
        let _ = self.it().verify_session().await;
        if let Err(e) = self.load_channels().await {
            // A signed-out or stale session must not replace the one the
            // user had: it was already saved by the rotation hook, and after a
            // restart Encore was stuck on "Choose a channel" with none.
            self.it().set_session(previous.clone());
            *self.expired.lock() = was_expired;
            self.channels.lock().clear();
            let _ = match &previous {
                Some(p) => save_session(self.store.as_ref(), p),
                None => self.store.delete(),
            };
            if previous.is_some() {
                let _ = self.load_channels().await;
            }
            return Err(e);
        }
        let s = self.it().session().ok_or_else(AppError::auth_required)?;
        if let Err(e) = save_session(self.store.as_ref(), &s) {
            // Never keep a session that can't be stored safely (no plaintext fallback).
            self.it().set_session(None);
            self.channels.lock().clear();
            return Err(store_err(e));
        }
        Ok(self.state())
    }

    pub async fn load_channels(&self) -> Result<Vec<Channel>> {
        // accounts_list must be asked without a stale page id.
        if let Some(mut s) = self.it().session() {
            if s.channel.is_some() {
                let keep = s.channel.take();
                self.it().set_session(Some(s.clone()));
                let r = self.ytm.accounts().await;
                s.channel = keep;
                self.it().set_session(Some(s));
                let list = r?;
                let chans: Vec<(Channel, ChannelSelection)> = list.into_iter().map(|a| (a.channel, a.selection)).collect();
                *self.channels.lock() = chans.clone();
                return Ok(chans.into_iter().map(|c| c.0).collect());
            }
        }
        let list = self.ytm.accounts().await?;
        let chans: Vec<(Channel, ChannelSelection)> = list.into_iter().map(|a| (a.channel, a.selection)).collect();
        *self.channels.lock() = chans.clone();
        Ok(chans.into_iter().map(|c| c.0).collect())
    }

    pub fn channels(&self) -> Vec<Channel> {
        self.channels.lock().iter().map(|c| c.0.clone()).collect()
    }

    /// Pick a channel (by key "authUser:pageId" or list index), validate it
    /// against a private surface and save.
    pub async fn select_channel(&self, key: &str) -> Result<AuthState> {
        if self.channels.lock().is_empty() {
            self.load_channels().await?;
        }
        let sel = {
            let ch = self.channels.lock();
            ch.iter().find(|c| c.0.key == key).or_else(|| key.parse::<usize>().ok().and_then(|i| ch.get(i))).map(|c| c.1.clone())
        }
        .ok_or_else(|| AppError::invalid("That channel is not in the list."))?;
        let mut s = self.it().session().ok_or_else(AppError::auth_required)?;
        let previous = s.channel.clone();
        s.channel = Some(sel);
        self.it().set_session(Some(s.clone()));
        match self.ytm.validate().await {
            Ok(_) => {
                s = self.it().session().unwrap_or(s);
                s.verified_at = Some(encore_core::store::now());
                self.it().set_session(Some(s.clone()));
                save_session(self.store.as_ref(), &s).map_err(store_err)?;
                *self.expired.lock() = None;
                Ok(self.state())
            }
            Err(e) => {
                s.channel = previous;
                self.it().set_session(Some(s));
                Err(e)
            }
        }
    }

    pub fn sign_out(&self) -> Result<()> {
        self.store.delete().map_err(store_err)?;
        self.it().set_session(None);
        self.channels.lock().clear();
        *self.expired.lock() = None;
        *self.refreshed.lock() = None;
        self.follow_paused.store(false, Ordering::Relaxed);
        self.follow_denied.store(false, Ordering::Relaxed);
        Ok(())
    }

    /// One keep-alive round (D8, D38): bring the cookies up to date when
    /// due (rotate an in-app sign-in, follow the browser an import came
    /// from), then `verify_session`. A refusal is checked with a forced
    /// refresh before the session counts as expired, and an expired session
    /// that works again (the browser signed in anew) is taken back.
    pub async fn keepalive(&self) -> Result<bool> {
        if !self.it().has_cookies() {
            return Ok(false);
        }
        let refreshed = self.refresh(false).await;
        match self.it().verify_session().await {
            Ok(changed) => {
                if changed {
                    if let Some(s) = self.it().session() {
                        let _ = save_session(self.store.as_ref(), &s);
                    }
                }
                let expired = self.expired.lock().clone();
                if let Some(reason) = expired {
                    self.recover(&reason).await;
                }
                Ok(changed || refreshed)
            }
            // `recover` marks the session expired when it is; no answer
            // marks nothing.
            Err(e) if e.code == ErrorCode::SessionExpired => match self.recover("YouTube Music signed this session out.").await {
                Recovery::Working => Ok(true),
                _ => Err(e),
            },
            Err(e) => Err(e),
        }
    }

    /// Bring the session's cookies up to date (D38). Scheduled, it runs when
    /// the last refresh is older than its interval; `force` (a request was
    /// refused) runs it unless one ran in the last minute. Returns whether
    /// the cookies changed.
    pub async fn refresh(&self, force: bool) -> bool {
        let _one = self.refreshing.lock().await;
        let epoch = self.it().epoch();
        let Some(s) = self.it().session() else { return false };
        let Some(how) = upkeep(&s.source) else { return false };
        let every = match how {
            Upkeep::Rotate => ROTATE_EVERY,
            Upkeep::Follow(_) if self.follow_denied.load(Ordering::Relaxed) => return false,
            Upkeep::Follow(_) if !force && self.follow_paused.load(Ordering::Relaxed) => return false,
            Upkeep::Follow(_) => FOLLOW_EVERY,
        };
        let last = *self.refreshed.lock();
        if last.is_some_and(|t| t.elapsed() < if force { FORCED_GAP } else { every }) {
            return false;
        }
        *self.refreshed.lock() = Some(Instant::now());
        match how {
            Upkeep::Rotate => match self.it().rotate_cookies().await {
                Ok(changed) => changed,
                Err(e) => {
                    tracing::info!(code = ?e.code, "session rotation failed");
                    false
                }
            },
            Upkeep::Follow(profile) => {
                // Scheduled, don't wait on a Keychain prompt; a forced one
                // gives the user time to answer it.
                let wait = Duration::from_secs(if force { 60 } else { 5 });
                let r = tokio::task::spawn_blocking(move || import::import(&profile, wait)).await;
                match r {
                    Ok(Ok(jar)) => {
                        self.follow_paused.store(false, Ordering::Relaxed);
                        let changed = self.it().replace_jar(jar, epoch);
                        tracing::info!(changed, "session read again from the browser");
                        changed
                    }
                    Ok(Err(e)) => {
                        if matches!(e, ImportError::KeychainDenied(_) | ImportError::KeyUnavailable(_)) {
                            let flag = if force { &self.follow_denied } else { &self.follow_paused };
                            flag.store(true, Ordering::Relaxed);
                            // Nothing was read, so a forced read may follow
                            // at once.
                            *self.refreshed.lock() = last;
                        }
                        tracing::info!(error = %e, "reading the session from the browser again failed");
                        false
                    }
                    Err(_) => false,
                }
            }
        }
    }

    /// A signed-in request was refused, or the session was marked expired:
    /// refresh it and try a private page before telling the user (D38).
    /// Working clears the expired mark; Expired sets it with `reason`. When
    /// the identity changed meanwhile (a sign-out, a new import or channel)
    /// the answer is about a session that is gone: Unknown, nothing marked.
    pub async fn recover(&self, reason: &str) -> Recovery {
        let epoch = self.it().epoch();
        if !self.it().signed_in() {
            return Recovery::Unknown;
        }
        self.refresh(true).await;
        let _ = self.it().verify_session().await;
        let r = self.ytm.validate().await;
        let mut expired = self.expired.lock();
        if self.it().epoch() != epoch {
            return Recovery::Unknown;
        }
        match r {
            Ok(_) => {
                *expired = None;
                Recovery::Working
            }
            Err(e) if e.code == ErrorCode::SessionExpired => {
                *expired = Some(reason.to_string());
                Recovery::Expired
            }
            Err(_) => Recovery::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upkeep_follows_the_source() {
        assert_eq!(upkeep("webview"), Some(Upkeep::Rotate));
        assert_eq!(upkeep("edge:Default"), Some(Upkeep::Follow("edge:Default".into())));
        assert_eq!(upkeep("firefox:abc.default-release"), Some(Upkeep::Follow("firefox:abc.default-release".into())));
        assert_eq!(upkeep("chrome:"), None);
        assert_eq!(upkeep("netscape:Default"), None);
        assert_eq!(upkeep("test"), None);
    }
}
