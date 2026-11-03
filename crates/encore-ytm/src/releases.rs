//! An artist's releases (D35): the pages behind the artist's "Albums" and
//! "Singles and EPs" links (`MPAD…` plus the shelf's own `params`), read
//! as given and page by page. Callers pace the continuation pages through
//! the background lane (`encore_core::paging`).

use encore_core::error::{AppError, Result};
use encore_core::model::{CollectionCard, PageLink};
use encore_core::service::{More, RequestContext};
use serde_json::json;

use crate::{CursorKind, YtMusic, pages, parse_err};

impl YtMusic {
    /// The first page of a release list. `link` must be one the service
    /// gave (an artist shelf's `more`); its id and params are never built.
    pub async fn releases(&self, ctx: &RequestContext, link: &PageLink) -> Result<(Vec<CollectionCard>, More)> {
        self.check(ctx)?;
        if !link.browse_id.starts_with("MPAD") {
            return Err(AppError::invalid("That is not a list of releases."));
        }
        let v = self.browse(&link.browse_id, link.params.as_deref()).await?;
        let p = pages::releases(&v).map_err(parse_err)?;
        let more = match self.cursors.put(CursorKind::Releases, p.cont, ctx.epoch) {
            Some(c) => More::Next(c),
            None => More::Ended,
        };
        Ok((p.value, more))
    }

    /// The next page, telling a real end from one that stopped making
    /// progress (`More::Stalled`), as `collection_more_checked` does.
    pub async fn releases_more(&self, ctx: &RequestContext, cursor: &str) -> Result<(Vec<CollectionCard>, More)> {
        self.check(ctx)?;
        let c = self.cursors.take(cursor, ctx.epoch)?;
        let CursorKind::Releases = c.kind else { return Err(AppError::invalid("Not a releases cursor.")) };
        let v = self.continue_browse("browse", &c.cont, json!({})).await?;
        let p = pages::releases_continuation(&v).map_err(parse_err)?;
        let same = matches!((&p.cont, &c.cont), (Some(a), b) if a == b);
        let more = if same || (p.value.is_empty() && p.cont.is_some()) {
            More::Stalled
        } else {
            match self.cursors.put(CursorKind::Releases, p.cont, ctx.epoch) {
                Some(next) => More::Next(next),
                None => More::Ended,
            }
        };
        Ok((p.value, more))
    }
}
