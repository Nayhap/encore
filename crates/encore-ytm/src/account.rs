//! Account and session management on top of InnerTube: browser import,
//! channel selection, validation, persistence in the OS credential store
//! (D9) and keep-alive.
//!
//! A session is saved right after import with no channel
//! (state `ChoosingChannel`); it becomes `SignedIn` only after the user picks
//! a channel and a private surface loads as that channel (D7, Meiro #1).

use std::sync::Arc;
use std::time::Duration;

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
}

fn store_err(e: encore_auth::store::StoreError) -> AppError {
    AppError::new(ErrorCode::Keychain, e.to_string())
}

impl Accounts {
    pub fn new(ytm: Arc<YtMusic>, store: Arc<dyn SecretStore>) -> Arc<Self> {
        let a = Arc::new(Self { ytm, store, channels: Mutex::new(vec![]), expired: Mutex::new(None) });
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
        Ok(())
    }

    /// One keep-alive round (D8): verify_session + rotation.
    pub async fn keepalive(&self) -> Result<bool> {
        if !self.it().has_cookies() {
            return Ok(false);
        }
        match self.it().verify_session().await {
            Ok(changed) => {
                if changed {
                    if let Some(s) = self.it().session() {
                        let _ = save_session(self.store.as_ref(), &s);
                    }
                }
                Ok(changed)
            }
            Err(e) if e.code == ErrorCode::SessionExpired => {
                self.mark_expired("YouTube Music signed this session out.");
                Err(e)
            }
            Err(e) => Err(e),
        }
    }
}
