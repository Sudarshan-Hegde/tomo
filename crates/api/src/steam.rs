//! Steam's public store endpoints (no API key needed): search, a popular list, and ID -> name lookups.
//! Results are cached in memory so we stay polite to Steam and stay fast.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::Deserialize;
use tomo_shared::{is_valid_banner_path, Game};

/// How many games the "popular right now" shelf shows.
pub const POPULAR_COUNT: usize = 36;
const POPULAR_TTL: Duration = Duration::from_secs(60 * 60);
const SEARCH_TTL: Duration = Duration::from_secs(10 * 60);
const CACHE_MAX_ENTRIES: usize = 500;
/// Steam's GetItems takes many ids at once; keep each request modest.
const LOOKUP_CHUNK: usize = 50;
const NAME_MAX_CHARS: usize = 300;

#[derive(Debug, PartialEq, Eq)]
pub enum SteamError {
    Unavailable,
}

pub struct SteamClient {
    http: reqwest::Client,
    store_base: String,
    api_base: String,
    cache: Mutex<HashMap<String, (Instant, Vec<Game>)>>,
}

impl SteamClient {
    pub fn new(http: reqwest::Client, store_base: String, api_base: String) -> Self {
        Self {
            http,
            store_base,
            api_base,
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn cached(&self, key: &str, ttl: Duration) -> Option<Vec<Game>> {
        let cache = self.cache.lock().unwrap();
        cache
            .get(key)
            .filter(|(at, _)| at.elapsed() < ttl)
            .map(|(_, games)| games.clone())
    }

    fn remember(&self, key: String, games: &[Game]) {
        let mut cache = self.cache.lock().unwrap();
        if cache.len() >= CACHE_MAX_ENTRIES {
            cache.clear(); // crude but bounded; entries are cheap to refetch
        }
        cache.insert(key, (Instant::now(), games.to_vec()));
    }

    /// The most-played games on Steam right now (software like Wallpaper Engine is filtered out).
    pub async fn popular(&self) -> Result<Vec<Game>, SteamError> {
        if let Some(hit) = self.cached("popular", POPULAR_TTL) {
            return Ok(hit);
        }
        let body = self
            .get_text(
                &format!(
                    "{}/ISteamChartsService/GetMostPlayedGames/v1/",
                    self.api_base
                ),
                &[],
            )
            .await?;
        let ids = parse_chart_ids(&body);
        let mut games = self.lookup(&ids).await?;
        games.truncate(POPULAR_COUNT);
        self.remember("popular".into(), &games);
        Ok(games)
    }

    /// Search Steam's catalogue by name. Only real games come back (not DLC, tools or hardware).
    pub async fn search(&self, query: &str) -> Result<Vec<Game>, SteamError> {
        let key = format!("search:{}", query.to_lowercase());
        if let Some(hit) = self.cached(&key, SEARCH_TTL) {
            return Ok(hit);
        }
        let body = self
            .get_text(
                &format!("{}/api/storesearch/", self.store_base),
                &[("term", query), ("l", "english"), ("cc", "us")],
            )
            .await?;
        let ids = parse_search_ids(&body);
        let games = self.lookup(&ids).await?;
        self.remember(key, &games);
        Ok(games)
    }

    /// Turn ids into names, keeping only games, in the order given.
    async fn lookup(&self, ids: &[u32]) -> Result<Vec<Game>, SteamError> {
        let mut found: HashMap<u32, Game> = HashMap::new();
        for chunk in ids.chunks(LOOKUP_CHUNK) {
            let input = serde_json::json!({
                "ids": chunk.iter().map(|id| serde_json::json!({ "appid": id })).collect::<Vec<_>>(),
                "context": { "language": "english", "country_code": "US" },
                "data_request": { "include_assets": true },
            })
            .to_string();
            let body = self
                .get_text(
                    &format!("{}/IStoreBrowseService/GetItems/v1/", self.api_base),
                    &[("input_json", input.as_str())],
                )
                .await?;
            for game in parse_store_items(&body) {
                found.insert(game.app_id, game);
            }
        }
        Ok(ids.iter().filter_map(|id| found.remove(id)).collect())
    }

    async fn get_text(&self, url: &str, query: &[(&str, &str)]) -> Result<String, SteamError> {
        let resp = self.http.get(url).query(query).send().await.map_err(|e| {
            tracing::warn!("steam request failed: {e}");
            SteamError::Unavailable
        })?;
        if !resp.status().is_success() {
            tracing::warn!("steam returned HTTP {}", resp.status());
            return Err(SteamError::Unavailable);
        }
        resp.text().await.map_err(|e| {
            tracing::warn!("steam response unreadable: {e}");
            SteamError::Unavailable
        })
    }
}

// ───────── parsing (pure, so it is easy to test) ─────────

#[derive(Deserialize)]
struct ChartEnvelope {
    response: ChartResponse,
}
#[derive(Deserialize)]
struct ChartResponse {
    #[serde(default)]
    ranks: Vec<ChartRank>,
}
#[derive(Deserialize)]
struct ChartRank {
    appid: u32,
}

/// App ids from the most-played chart, best first.
pub fn parse_chart_ids(json: &str) -> Vec<u32> {
    serde_json::from_str::<ChartEnvelope>(json)
        .map(|c| c.response.ranks.into_iter().map(|r| r.appid).collect())
        .unwrap_or_default()
}

#[derive(Deserialize)]
struct SearchEnvelope {
    #[serde(default)]
    items: Vec<SearchItem>,
}
#[derive(Deserialize)]
struct SearchItem {
    #[serde(default)]
    id: u32,
    #[serde(default, rename = "type")]
    kind: String,
}

/// App ids from a store search, in Steam's order. Bundles and packages are skipped.
pub fn parse_search_ids(json: &str) -> Vec<u32> {
    serde_json::from_str::<SearchEnvelope>(json)
        .map(|s| {
            s.items
                .into_iter()
                .filter(|i| i.kind == "app" && i.id > 0)
                .map(|i| i.id)
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Deserialize)]
struct ItemsEnvelope {
    response: ItemsResponse,
}
#[derive(Deserialize)]
struct ItemsResponse {
    #[serde(default)]
    store_items: Vec<StoreItem>,
}
#[derive(Deserialize)]
struct StoreItem {
    #[serde(default)]
    appid: u32,
    #[serde(default)]
    name: String,
    /// 0 = game. Other numbers are software, DLC, tools, hardware and so on.
    #[serde(default, rename = "type")]
    kind: i64,
    #[serde(default = "yes")]
    visible: bool,
    #[serde(default)]
    assets: Option<StoreAssets>,
}
#[derive(Deserialize)]
struct StoreAssets {
    #[serde(default)]
    header: Option<String>,
}
fn yes() -> bool {
    true
}

/// Games (and only games) out of a GetItems response.
pub fn parse_store_items(json: &str) -> Vec<Game> {
    serde_json::from_str::<ItemsEnvelope>(json)
        .map(|r| {
            r.response
                .store_items
                .into_iter()
                .filter(|i| i.kind == 0 && i.visible && i.appid > 0 && !i.name.trim().is_empty())
                .map(|i| Game {
                    app_id: i.appid,
                    name: i.name.trim().chars().take(NAME_MAX_CHARS).collect(),
                    // Only a banner path of exactly the expected shape is ever kept.
                    banner: i
                        .assets
                        .and_then(|a| a.header)
                        .filter(|h| is_valid_banner_path(h)),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Query, routing::get, Json, Router};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[test]
    fn chart_ids_keep_steams_order() {
        let json = r#"{"response":{"rollup_date":1,"ranks":[{"rank":1,"appid":730,"peak_in_game":9},{"rank":2,"appid":570}]}}"#;
        assert_eq!(parse_chart_ids(json), vec![730, 570]);
        assert!(parse_chart_ids("not json").is_empty());
        assert!(parse_chart_ids(r#"{"response":{}}"#).is_empty());
    }

    #[test]
    fn search_skips_bundles_and_zero_ids() {
        let json = r#"{"total":3,"items":[{"type":"app","id":730,"name":"CS2"},{"type":"sub","id":99,"name":"Bundle"},{"type":"app","id":0}]}"#;
        assert_eq!(parse_search_ids(json), vec![730]);
    }

    #[test]
    fn only_games_survive_the_lookup() {
        let json = r#"{"response":{"store_items":[
            {"appid":730,"name":"Counter-Strike 2","type":0,"visible":true},
            {"appid":431960,"name":"Wallpaper Engine","type":6,"visible":true},
            {"appid":4165890,"name":"Steam Frame","type":7},
            {"appid":5,"type":0},
            {"appid":6,"name":"  Hidden  ","type":0,"visible":false},
            {"appid":570,"name":"  Dota 2  ","type":0}
        ]}}"#;
        let games = parse_store_items(json);
        assert_eq!(
            games,
            vec![
                Game {
                    app_id: 730,
                    name: "Counter-Strike 2".into(),
                    banner: None
                },
                Game {
                    app_id: 570,
                    name: "Dota 2".into(),
                    banner: None
                }
            ]
        );
    }

    #[test]
    fn steams_banner_path_is_kept_only_when_it_has_the_expected_shape() {
        let hash = "db8e0cf43c9475b96fb9446b4908de79fcb9f619";
        let json = format!(
            r#"{{"response":{{"store_items":[
                {{"appid":1,"name":"Good","type":0,"assets":{{"header":"{hash}/header.jpg"}}}},
                {{"appid":2,"name":"Plain","type":0,"assets":{{"header":"header.jpg"}}}},
                {{"appid":3,"name":"Hostile","type":0,"assets":{{"header":"../../evil.jpg"}}}},
                {{"appid":4,"name":"Absolute","type":0,"assets":{{"header":"https://evil.example/header.jpg"}}}},
                {{"appid":5,"name":"No assets","type":0}}
            ]}}}}"#
        );
        let banners: Vec<_> = parse_store_items(&json)
            .into_iter()
            .map(|g| g.banner)
            .collect();
        assert_eq!(
            banners,
            vec![
                Some(format!("{hash}/header.jpg")),
                Some("header.jpg".into()),
                None,
                None,
                None
            ]
        );
    }

    #[test]
    fn very_long_names_are_cut() {
        let json = format!(
            r#"{{"response":{{"store_items":[{{"appid":1,"name":"{}","type":0}}]}}}}"#,
            "x".repeat(900)
        );
        assert_eq!(
            parse_store_items(&json)[0].name.chars().count(),
            NAME_MAX_CHARS
        );
    }

    /// A stand-in for Steam. Counts requests so caching can be checked.
    async fn spawn_fake_steam(hits: Arc<AtomicUsize>) -> String {
        let h1 = hits.clone();
        let h2 = hits.clone();
        let h3 = hits;
        let app = Router::new()
            .route(
                "/ISteamChartsService/GetMostPlayedGames/v1/",
                get(move || {
                    let h = h1.clone();
                    async move {
                        h.fetch_add(1, Ordering::SeqCst);
                        Json(serde_json::json!({"response":{"ranks":[{"appid":730},{"appid":431960},{"appid":570}]}}))
                    }
                }),
            )
            .route(
                "/api/storesearch/",
                get(move |Query(q): Query<HashMap<String, String>>| {
                    let h = h2.clone();
                    async move {
                        h.fetch_add(1, Ordering::SeqCst);
                        let term = q.get("term").cloned().unwrap_or_default();
                        let items = if term.to_lowercase().contains("counter") {
                            serde_json::json!([{"type":"app","id":10},{"type":"app","id":730},{"type":"app","id":4453300}])
                        } else {
                            serde_json::json!([])
                        };
                        Json(serde_json::json!({"total": 0, "items": items}))
                    }
                }),
            )
            .route(
                "/IStoreBrowseService/GetItems/v1/",
                get(move |Query(q): Query<HashMap<String, String>>| {
                    let h = h3.clone();
                    async move {
                        h.fetch_add(1, Ordering::SeqCst);
                        let input: serde_json::Value = serde_json::from_str(&q["input_json"]).unwrap();
                        let items: Vec<serde_json::Value> = input["ids"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|i| i["appid"].as_u64().unwrap())
                            .map(|id| match id {
                                730 => serde_json::json!({"appid":730,"name":"Counter-Strike 2","type":0}),
                                10 => serde_json::json!({"appid":10,"name":"Counter-Strike","type":0}),
                                570 => serde_json::json!({"appid":570,"name":"Dota 2","type":0}),
                                431960 => serde_json::json!({"appid":431960,"name":"Wallpaper Engine","type":6}),
                                other => serde_json::json!({"appid":other,"name":"Counter Cup DLC","type":4}),
                            })
                            .collect();
                        // Steam does not promise the order of its answer, so reverse it on purpose.
                        let mut items = items;
                        items.reverse();
                        Json(serde_json::json!({"response":{"store_items": items}}))
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn client(base: &str) -> SteamClient {
        SteamClient::new(reqwest::Client::new(), base.into(), base.into())
    }

    #[tokio::test]
    async fn popular_lists_real_games_in_chart_order_without_software() {
        let hits = Arc::new(AtomicUsize::new(0));
        let base = spawn_fake_steam(hits).await;
        let games = client(&base).popular().await.unwrap();
        let names: Vec<_> = games.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Counter-Strike 2", "Dota 2"],
            "Wallpaper Engine must be filtered out"
        );
    }

    #[tokio::test]
    async fn search_returns_games_only_in_steams_order() {
        let hits = Arc::new(AtomicUsize::new(0));
        let base = spawn_fake_steam(hits).await;
        let games = client(&base).search("counter").await.unwrap();
        let names: Vec<_> = games.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Counter-Strike", "Counter-Strike 2"],
            "DLC dropped, search order kept"
        );
        assert!(client(&base)
            .search("zzzz nothing")
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn repeat_requests_are_served_from_the_cache() {
        let hits = Arc::new(AtomicUsize::new(0));
        let base = spawn_fake_steam(hits.clone()).await;
        let steam = client(&base);
        steam.popular().await.unwrap();
        let after_first = hits.load(Ordering::SeqCst);
        steam.popular().await.unwrap();
        steam.popular().await.unwrap();
        assert_eq!(
            hits.load(Ordering::SeqCst),
            after_first,
            "no new calls to Steam"
        );
        steam.search("Counter").await.unwrap();
        let after_search = hits.load(Ordering::SeqCst);
        steam.search("COUNTER").await.unwrap();
        assert_eq!(
            hits.load(Ordering::SeqCst),
            after_search,
            "search cache ignores letter case"
        );
    }

    #[tokio::test]
    async fn an_unreachable_steam_is_reported_not_hidden() {
        let steam = client("http://127.0.0.1:1");
        assert_eq!(steam.popular().await, Err(SteamError::Unavailable));
        assert_eq!(steam.search("counter").await, Err(SteamError::Unavailable));
    }
}
