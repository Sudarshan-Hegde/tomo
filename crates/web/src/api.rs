//! Thin client for the Tomo API plus browser-side session storage.

use gloo_net::http::{Request, Response};
use leptos::prelude::*;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tomo_shared::{ErrorBody, User};

/// Dev default. Moves to build-time config when we deploy.
pub const API_URL: &str = "http://localhost:8080";
const SESSION_KEY: &str = "tomo-session";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub user: User,
}

/// App-wide signal holding the signed-in session, if any.
pub type SessionSignal = RwSignal<Option<Session>>;

#[derive(Debug, Clone)]
pub enum ApiErr {
    Unauthorized,
    /// HTTP 409: the request is valid but blocked by state (e.g. no profile yet).
    Conflict(String),
    Msg(String),
}

impl ApiErr {
    pub fn text(&self) -> String {
        match self {
            ApiErr::Unauthorized => "Your session expired. Please sign in again.".into(),
            ApiErr::Conflict(m) | ApiErr::Msg(m) => m.clone(),
        }
    }
}

// ───────── Session persistence ─────────

pub fn load_session() -> Option<Session> {
    let raw = window().local_storage().ok().flatten()?.get_item(SESSION_KEY).ok().flatten()?;
    serde_json::from_str(&raw).ok()
}

pub fn store_session(session: Option<&Session>) {
    let Some(storage) = window().local_storage().ok().flatten() else { return };
    match session.and_then(|s| serde_json::to_string(s).ok()) {
        Some(json) => {
            let _ = storage.set_item(SESSION_KEY, &json);
        }
        None => {
            let _ = storage.remove_item(SESSION_KEY);
        }
    }
}

/// Turn an API error into a message; an expired session also signs the user out.
pub fn handle_err(session: SessionSignal, e: ApiErr) -> String {
    if matches!(e, ApiErr::Unauthorized) {
        store_session(None);
        session.set(None);
    }
    e.text()
}

// ───────── HTTP ─────────

fn auth(req: gloo_net::http::RequestBuilder, token: Option<&str>) -> gloo_net::http::RequestBuilder {
    match token {
        Some(t) => req.header("Authorization", &format!("Bearer {t}")),
        None => req,
    }
}

async fn finish<T: DeserializeOwned>(resp: Response) -> Result<T, ApiErr> {
    if resp.status() == 401 {
        return Err(ApiErr::Unauthorized);
    }
    if !resp.ok() {
        let status = resp.status();
        let msg = resp
            .json::<ErrorBody>()
            .await
            .map(|e| e.error)
            .unwrap_or_else(|_| format!("Request failed (HTTP {status})."));
        return Err(if status == 409 { ApiErr::Conflict(msg) } else { ApiErr::Msg(msg) });
    }
    resp.json::<T>().await.map_err(|e| ApiErr::Msg(e.to_string()))
}

fn net_err(e: gloo_net::Error) -> ApiErr {
    ApiErr::Msg(format!("Cannot reach the server ({e}). Is the API running?"))
}

pub async fn get<T: DeserializeOwned>(path: &str, token: Option<&str>) -> Result<T, ApiErr> {
    let resp = auth(Request::get(&format!("{API_URL}{path}")), token).send().await.map_err(net_err)?;
    finish(resp).await
}

pub async fn post<B: Serialize, T: DeserializeOwned>(path: &str, token: Option<&str>, body: &B) -> Result<T, ApiErr> {
    let req = auth(Request::post(&format!("{API_URL}{path}")), token)
        .json(body)
        .map_err(|e| ApiErr::Msg(e.to_string()))?;
    finish(req.send().await.map_err(net_err)?).await
}

pub async fn put<B: Serialize, T: DeserializeOwned>(path: &str, token: Option<&str>, body: &B) -> Result<T, ApiErr> {
    let req = auth(Request::put(&format!("{API_URL}{path}")), token)
        .json(body)
        .map_err(|e| ApiErr::Msg(e.to_string()))?;
    finish(req.send().await.map_err(net_err)?).await
}

/// For calls that reply with an empty body (block, unblock, report).
async fn finish_unit(resp: Response) -> Result<(), ApiErr> {
    if resp.ok() {
        return Ok(());
    }
    finish::<serde_json::Value>(resp).await.map(|_| ())
}

pub async fn post_unit<B: Serialize>(path: &str, token: Option<&str>, body: &B) -> Result<(), ApiErr> {
    let req = auth(Request::post(&format!("{API_URL}{path}")), token)
        .json(body)
        .map_err(|e| ApiErr::Msg(e.to_string()))?;
    finish_unit(req.send().await.map_err(net_err)?).await
}

pub async fn delete_unit(path: &str, token: Option<&str>) -> Result<(), ApiErr> {
    let resp = auth(Request::delete(&format!("{API_URL}{path}")), token).send().await.map_err(net_err)?;
    finish_unit(resp).await
}

/// ws:// (or wss://) address of the chat socket, derived from the API address.
pub fn ws_url() -> String {
    format!("{}/ws", API_URL.replacen("http", "ws", 1))
}

/// Best-effort server-side logout; the local session is cleared regardless.
pub async fn logout(token: &str) {
    let _ = auth(Request::post(&format!("{API_URL}/auth/logout")), Some(token)).send().await;
}
