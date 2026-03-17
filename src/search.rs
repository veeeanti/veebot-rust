//! Search module for veebot
//! Handles UnionCrax game search and web search

use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SearchError {
    #[error("HTTP request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),
    #[error("Parse error: {0}")]
    ParseError(String),
    #[error("No results found")]
    NoResults,
}

pub type SearchResult<T> = Result<T, SearchError>;

/// UnionCrax game info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameInfo {
    pub title: String,
    pub url: String,
    pub description: Option<String>,
    pub source: String,
    pub download_count: Option<u64>,
    pub view_count: Option<u64>,
    pub size: Option<String>,
    pub appid: Option<String>,
}

/// UnionCrax API game response
#[derive(Debug, Deserialize)]
struct UnionCraxGame {
    name: Option<String>,
    description: Option<String>,
    appid: Option<String>,
    id: Option<String>,
    size: Option<String>,
    source: Option<String>,
}

/// Download stats from UnionCrax
#[derive(Debug, Deserialize)]
struct DownloadStats(serde_json::Value);

/// Web search result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    pub description: String,
    pub source: String,
}

/// Search service
pub struct SearchService {
    client: Client,
    union_crax_base: String,
    search_engine: String,
}

impl SearchService {
    /// Create a new search service
    pub fn new(search_engine: String) -> Self {
        Self {
            client: Client::new(),
            union_crax_base: "https://union-crax.xyz".to_string(),
            search_engine,
        }
    }
    
    /// Normalize a string for search
    fn normalize_string(&self, s: &str) -> String {
        s.to_lowercase()
            .chars()
            .map(|c| c.to_ascii_lowercase())
            .collect::<String>()
            .split(|c: char| !c.is_alphanumeric() && c != ' ')
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string()
    }
    
    /// Search for games on UnionCrax
    pub async fn search_union_crax_games(&self, query: &str) -> SearchResult<Vec<GameInfo>> {
        let normalized_query = self.normalize_string(query);
        
        // Fetch games and stats in parallel
        let (games_resp, stats_resp) = tokio::join!(
            self.client
                .get(format!("{}/api/games", self.union_crax_base))
                .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
                .timeout(std::time::Duration::from_secs(10))
                .send(),
            self.client
                .get(format!("{}/api/downloads/all", self.union_crax_base))
                .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
                .timeout(std::time::Duration::from_secs(10))
                .send()
        );
        
        let games_data: Vec<UnionCraxGame> = match games_resp {
            Ok(resp) => resp.json().await.unwrap_or_default(),
            Err(e) => {
                tracing::warn!("Failed to fetch games: {}", e);
                vec![]
            }
        };
        
        let stats_data: serde_json::Value = match stats_resp {
            Ok(resp) => resp.json().await.unwrap_or(serde_json::Value::Null),
            Err(e) => {
                tracing::warn!("Failed to fetch stats: {}", e);
                serde_json::Value::Null
            }
        };
        
        if games_data.is_empty() {
            return Ok(vec![]);
        }
        
        let query_words: Vec<&str> = normalized_query
            .split_whitespace()
            .filter(|w| w.len() > 2)
            .collect();
        
        // Score games
        let mut scored_games: Vec<(UnionCraxGame, i32)> = games_data
            .into_iter()
            .map(|game| {
                let normalized_name = self.normalize_string(game.name.as_deref().unwrap_or(""));
                let normalized_desc = self.normalize_string(game.description.as_deref().unwrap_or(""));
                let mut score = 0;
                
                // Exact match
                if normalized_name == normalized_query {
                    score += 100;
                }
                // Contains match
                if normalized_name.contains(&normalized_query) && 
                   (normalized_name.len() as i32 - normalized_query.len() as i32).abs() < 10 {
                    score += 60;
                }
                // Query contains name
                if normalized_query.contains(&normalized_name) && normalized_name.len() > 4 {
                    score += 40;
                }
                
                // Word matching
                for word in &query_words {
                    if word.len() > 3 {
                        if normalized_name.contains(word) {
                            score += 25;
                        } else if normalized_desc.contains(word) {
                            score += 8;
                        }
                    }
                }
                
                // Prefix match
                if normalized_name.starts_with(&normalized_query) {
                    score += 30;
                }
                
                // AppID match
                let appid = game.appid.as_deref().or(game.id.as_deref()).unwrap_or("");
                if appid == normalized_query {
                    score += 20;
                }
                
                (game, score)
            })
            .collect();
        
        // Sort by score descending
        scored_games.sort_by(|a, b| b.1.cmp(&a.1));
        
        // Filter by threshold and take top 3
        let results: Vec<GameInfo> = scored_games
            .into_iter()
            .filter(|(_, score)| *score >= 60)
            .take(3)
            .map(|(game, _)| {
                let appid = game.appid.as_deref().or(game.id.as_deref()).unwrap_or("");
                let stats = stats_data.get(appid);
                
                GameInfo {
                    title: format!("{} - Free Download on UnionCrax", game.name.as_deref().unwrap_or("Unknown")),
                    url: format!("{}/game/{}", self.union_crax_base, appid),
                    description: game.description.clone(),
                    source: game.source.clone().unwrap_or_else(|| "UnionCrax".to_string()),
                    download_count: stats.and_then(|s| s.get("downloads").and_then(|v| v.as_u64())),
                    view_count: stats.and_then(|s| s.get("views").and_then(|v| v.as_u64())),
                    size: game.size.clone(),
                    appid: game.appid.or(game.id),
                }
            })
            .collect();
        
        Ok(results)
    }
    
    /// Search Google for UnionCrax games
    pub async fn search_google_for_union_crax(&self, query: &str) -> SearchResult<Option<GameInfo>> {
        let games = self.search_union_crax_games(query).await?;
        
        if games.is_empty() {
            return Err(SearchError::NoResults);
        }
        
        let top_game = &games[0];
        
        // Return the game (we skip the Google verification step for simplicity)
        Ok(Some(top_game.clone()))
    }
    
    /// Perform a web search
    pub async fn perform_web_search(&self, query: &str) -> SearchResult<Vec<WebSearchResult>> {
        let search_url = format!("{}{}", self.search_engine, urlencoding::encode(query));
        
        let response = self.client
            .get(&search_url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;
        
        let html = response.text().await?;
        
        // Simple HTML parsing to extract search results
        // Note: In production, you'd want to use a proper HTML parser like scraper
        let mut results = Vec::new();
        
        // Look for result divs
        for line in html.lines() {
            if line.contains("class=\"g\"") || line.contains("class='g'") {
                // This is a simplified parser - real implementation would need proper HTML parsing
                results.push(WebSearchResult {
                    title: "Search result".to_string(),
                    url: search_url.clone(),
                    description: format!("Results for {}", query),
                    source: "Web Search".to_string(),
                });
            }
        }
        
        // Fallback if no results found
        if results.is_empty() {
            results.push(WebSearchResult {
                title: format!("Search results for \"{}\"", query),
                url: search_url,
                description: format!("Find information about {} on the web", query),
                source: "Web Search".to_string(),
            });
        }
        
        Ok(results)
    }
}

// URL encoding helper
mod urlencoding {
    pub fn encode(s: &str) -> String {
        let mut result = String::new();
        for c in s.chars() {
            match c {
                'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => result.push(c),
                ' ' => result.push_str("%20"),
                _ => {
                    for byte in c.to_string().as_bytes() {
                        result.push_str(&format!("%{:02X}", byte));
                    }
                }
            }
        }
        result
    }
}
