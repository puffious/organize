use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde::Deserialize;

use crate::config::MediaType;

pub trait MetadataLookup {
    fn lookup_year(&self, title: &str, media_type: MediaType) -> Result<Option<u16>>;
}

pub struct TmdbClient {
    api_key: String,
    http: Client,
}

impl TmdbClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            http: Client::new(),
        }
    }
}

impl MetadataLookup for TmdbClient {
    fn lookup_year(&self, title: &str, media_type: MediaType) -> Result<Option<u16>> {
        if self.api_key.trim().is_empty() {
            return Ok(None);
        }

        let endpoint = match media_type {
            MediaType::Show => "https://api.themoviedb.org/3/search/tv",
            MediaType::Movie => "https://api.themoviedb.org/3/search/movie",
        };

        let response = self
            .http
            .get(endpoint)
            .query(&[("api_key", self.api_key.as_str()), ("query", title)])
            .send()
            .with_context(|| "tmdb request failed")?
            .error_for_status()
            .with_context(|| "tmdb returned non-success status")?;

        let body: SearchResponse = response.json().with_context(|| "invalid tmdb json")?;
        let first = body.results.into_iter().next();
        let date = match media_type {
            MediaType::Show => first.and_then(|v| v.first_air_date),
            MediaType::Movie => first.and_then(|v| v.release_date),
        };

        Ok(date.and_then(parse_year_from_date))
    }
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    results: Vec<SearchResult>,
}

#[derive(Debug, Deserialize)]
struct SearchResult {
    release_date: Option<String>,
    first_air_date: Option<String>,
}

fn parse_year_from_date(s: String) -> Option<u16> {
    s.get(0..4).and_then(|y| y.parse::<u16>().ok())
}
