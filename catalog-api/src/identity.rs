//! IAP identity middleware. Cloud IAP authenticates the caller and passes the verified email in
//! the `x-goog-authenticated-user-email` header, prefixed with the identity provider (e.g.
//! `accounts.google.com:alice@x.co`). This layer extracts and normalizes that email, exposes it
//! to handlers as a [`CallerIdentity`] request extension, and upserts the user into the catalog's
//! users table on the first sighting of each email this process sees.
//!
//! There is no auth enforcement here: IAP already gates the request at the edge. The header is
//! trusted because the platform sets it after verification and strips any client-supplied copy.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;

/// The header Cloud IAP populates with the authenticated caller's email.
pub const IAP_EMAIL_HEADER: &str = "x-goog-authenticated-user-email";

/// The caller's normalized (lowercased, prefix-stripped) email, or `None` when the request
/// carried no usable IAP identity. Inserted as a request extension by [`identity_layer`].
#[derive(Clone, Debug)]
pub struct CallerIdentity(pub Option<String>);

/// State for [`identity_layer`]: where to persist sightings, and the set of emails already
/// recorded by this process so a repeat sighting doesn't re-spawn a write.
#[derive(Clone)]
pub struct IdentityState {
    pub users_path: String,
    pub seen: Arc<Mutex<HashSet<String>>>,
}

impl IdentityState {
    pub fn new(users_path: String) -> Self {
        Self {
            users_path,
            seen: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

/// Axum middleware: extract the IAP email, attach it as a [`CallerIdentity`] extension, and on the
/// first sighting of an email this process fire-and-forget records it into the users table.
pub async fn identity_layer(
    axum::extract::State(st): axum::extract::State<IdentityState>,
    mut req: Request,
    next: Next,
) -> Response {
    let email = req
        .headers()
        .get(IAP_EMAIL_HEADER)
        .and_then(|v| v.to_str().ok())
        // The value is `<provider>:<email>`; take the part after the last `:`.
        .map(|v| {
            v.rsplit(':')
                .next()
                .unwrap_or(v)
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|e| !e.is_empty());
    if let Some(e) = &email {
        let first_sight = st.seen.lock().expect("seen set").insert(e.clone());
        if first_sight {
            let (path, email) = (st.users_path.clone(), e.clone());
            tokio::spawn(async move {
                if let Err(err) =
                    catalog_core::record_user_seen(&path, &email, chrono::Utc::now()).await
                {
                    tracing::warn!(error = %err, "failed to record user sighting");
                }
            });
        }
    }
    req.extensions_mut().insert(CallerIdentity(email));
    next.run(req).await
}
