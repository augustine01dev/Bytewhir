use chrono::{DateTime, Utc};
use reqwest::Client;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::task::JoinSet;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DigestItem {
    pub id: String,
    pub source: String,
    pub category: Option<String>,
    pub title: String,
    pub summary: Option<String>,
    pub thumbnail_url: Option<String>,
    pub url: String,
    pub published_at: String,
    pub fetched_at: String,
    pub seen: i32,
    pub starred: i32,
    #[serde(skip)]
    pub thumbnail_resolved: i32,
}

impl DigestItem {
    pub fn new(
        source: &str,
        category: Option<&str>,
        title: &str,
        url: &str,
        summary: Option<&str>,
        thumbnail_url: Option<&str>,
        published_at: &str,
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(url.as_bytes());
        let id = hex::encode(hasher.finalize());
        let fetched_at = chrono::Utc::now().to_rfc3339();

        Self {
            id,
            source: source.to_string(),
            category: category.map(|s| s.to_string()),
            title: title.to_string(),
            summary: summary.map(|s| s.to_string()),
            thumbnail_url: thumbnail_url.map(|s| s.to_string()),
            url: url.to_string(),
            published_at: published_at.to_string(),
            fetched_at,
            seen: 0,
            starred: 0,
            thumbnail_resolved: i32::from(thumbnail_url.is_some()),
        }
    }

    pub fn mark_thumbnail_resolved(&mut self) {
        self.thumbnail_resolved = 1;
    }
}

fn image_url_from_entry(entry: &feed_rs::model::Entry) -> Option<String> {
    entry
        .media
        .iter()
        .flat_map(|media| media.content.iter())
        .find(|content| {
            let is_image = content.content_type.as_ref().map_or(false, |content_type| {
                content_type.to_string().starts_with("image/")
            });
            is_image
                || content
                    .url
                    .as_ref()
                    .map_or(false, |url| is_probable_image_url(url.as_str()))
        })
        .and_then(|content| content.url.as_ref())
        .map(|url| url.to_string())
        .or_else(|| {
            entry
                .media
                .iter()
                .flat_map(|media| media.thumbnails.iter())
                .next()
                .map(|thumbnail| thumbnail.image.uri.clone())
        })
}

fn is_probable_image_url(url: &str) -> bool {
    let path = reqwest::Url::parse(url)
        .ok()
        .map(|parsed| parsed.path().to_ascii_lowercase())
        .unwrap_or_else(|| url.to_ascii_lowercase());

    [".jpg", ".jpeg", ".png", ".webp", ".gif", ".avif"]
        .iter()
        .any(|extension| path.contains(extension))
}

fn is_http_url(url: &str) -> bool {
    reqwest::Url::parse(url)
        .map(|parsed| matches!(parsed.scheme(), "http" | "https"))
        .unwrap_or(false)
}

async fn resolve_og_image_for_item(
    client: &Client,
    source: &str,
    item_title: &str,
    article_url: &str,
) -> Option<String> {
    if !is_http_url(article_url) {
        eprintln!(
            "[thumbnail][{source}] item={item_title:?} outcome=invalid_article_url url={article_url:?}"
        );
        return None;
    }

    let response = match client.get(article_url).send().await {
        Ok(response) => response,
        Err(error) if error.is_timeout() => {
            eprintln!(
                "[thumbnail][{source}] item={item_title:?} outcome=timeout url={article_url}"
            );
            return None;
        }
        Err(error) => {
            eprintln!(
                "[thumbnail][{source}] item={item_title:?} outcome=request_error error={error} url={article_url}"
            );
            return None;
        }
    };

    if !response.status().is_success() {
        eprintln!(
            "[thumbnail][{source}] item={item_title:?} outcome=non_200 status={} url={article_url}",
            response.status().as_u16()
        );
        return None;
    }

    let page_url = response.url().clone();
    let html = match response.text().await {
        Ok(html) => html,
        Err(error) if error.is_timeout() => {
            eprintln!(
                "[thumbnail][{source}] item={item_title:?} outcome=timeout_while_reading_response url={article_url}"
            );
            return None;
        }
        Err(error) => {
            eprintln!(
                "[thumbnail][{source}] item={item_title:?} outcome=response_read_error error={error} url={article_url}"
            );
            return None;
        }
    };

    let document = Html::parse_document(&html);
    let selectors = [
        "meta[property=\"og:image\"]",
        "meta[property=\"og:image:url\"]",
        "meta[name=\"og:image\"]",
    ];
    let mut found_image_content = false;

    for selector_text in selectors {
        let selector = Selector::parse(selector_text).expect("static OG image selector");
        for meta in document.select(&selector) {
            let Some(content) = meta.value().attr("content").map(str::trim) else {
                continue;
            };
            if content.is_empty() {
                continue;
            }
            found_image_content = true;

            if let Ok(image_url) = reqwest::Url::parse(content).or_else(|_| page_url.join(content))
            {
                if matches!(image_url.scheme(), "http" | "https") {
                    let image_url = image_url.to_string();
                    eprintln!(
                        "[thumbnail][{source}] item={item_title:?} outcome=success image_url={image_url}"
                    );
                    return Some(image_url);
                }
            }
        }
    }

    if found_image_content {
        eprintln!(
            "[thumbnail][{source}] item={item_title:?} outcome=invalid_image_tag url={article_url}"
        );
    } else {
        eprintln!(
            "[thumbnail][{source}] item={item_title:?} outcome=no_image_tag_found url={article_url}"
        );
    }

    None
}

fn parse_source_timestamp(source: &str, raw_value: &str) -> Option<String> {
    match DateTime::parse_from_rfc3339(raw_value) {
        Ok(parsed) => Some(parsed.with_timezone(&Utc).to_rfc3339()),
        Err(error) => {
            eprintln!(
                "[timestamp][{source}] raw={raw_value:?} parsed=None outcome=invalid error={error}"
            );
            None
        }
    }
}

fn log_timestamp_sample(source: &str, raw_value: &str, parsed_value: Option<&str>, index: usize) {
    if index < 3 {
        eprintln!("[timestamp][{source}] raw={raw_value:?} parsed={parsed_value:?}");
    }
}

fn github_preview_url(repository_url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(repository_url).ok()?;
    if parsed.host_str()? != "github.com" {
        return None;
    }

    let segments: Vec<_> = parsed
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.len() < 2 {
        return None;
    }

    Some(format!(
        "https://opengraph.githubassets.com/1/{}/{}",
        segments[0], segments[1]
    ))
}

fn is_external_story_url(story_url: &str) -> bool {
    let parsed = match reqwest::Url::parse(story_url) {
        Ok(parsed) => parsed,
        Err(_) => return false,
    };

    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }

    !matches!(
        parsed.host_str(),
        Some("news.ycombinator.com") | Some("ycombinator.com")
    )
}

fn image_client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(2))
        .user_agent("bytewhir/0.1 (thumbnail resolver)")
        .build()
        .map_err(|error| error.to_string())
}

#[derive(Deserialize)]
struct AlgoliaResponse {
    hits: Vec<AlgoliaHit>,
}

#[derive(Deserialize)]
struct AlgoliaHit {
    title: Option<String>,
    url: Option<String>,
    created_at: String,
}

pub async fn fetch_hn(client: &Client) -> Result<Vec<DigestItem>, String> {
    let url = "https://hn.algolia.com/api/v1/search_by_date?query=AI&tags=story&hitsPerPage=10";
    let resp: AlgoliaResponse = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let thumbnail_client = image_client()?;
    let mut tasks = JoinSet::new();

    for (index, hit) in resp.hits.into_iter().enumerate() {
        if let (Some(title), Some(url)) = (hit.title, hit.url) {
            let raw_created_at = hit.created_at.clone();
            let parsed_created_at = parse_source_timestamp("HN", &raw_created_at);
            log_timestamp_sample("HN", &raw_created_at, parsed_created_at.as_deref(), index);
            let Some(published_at) = parsed_created_at else {
                eprintln!("[timestamp][HN] item={title:?} outcome=skipped_invalid_timestamp");
                continue;
            };

            let thumbnail_client = thumbnail_client.clone();
            tasks.spawn(async move {
                let thumbnail_url = if is_external_story_url(&url) {
                    resolve_og_image_for_item(&thumbnail_client, "HN", &title, &url).await
                } else {
                    eprintln!(
                        "[thumbnail][HN] item={title:?} outcome=skipped_internal_story url={url}"
                    );
                    None
                };
                let mut item = DigestItem::new(
                    "Hacker News",
                    Some("Tech"),
                    &title,
                    &url,
                    None,
                    thumbnail_url.as_deref(),
                    &published_at,
                );
                item.mark_thumbnail_resolved();
                item
            });
        }
    }

    let mut items = Vec::new();
    while let Some(result) = tasks.join_next().await {
        if let Ok(item) = result {
            items.push(item);
        }
    }

    Ok(items)
}

pub async fn fetch_github(client: &Client) -> Result<Vec<DigestItem>, String> {
    let url = "https://github.com/trending?since=daily";
    let text = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let document = Html::parse_document(&text);
    let repo_selector = Selector::parse("article.Box-row").unwrap();
    let title_selector = Selector::parse("h2.h3 a").unwrap();
    let desc_selector = Selector::parse("p").unwrap();

    let mut items = Vec::new();
    let today = chrono::Utc::now().to_rfc3339();

    for element in document.select(&repo_selector) {
        let title_el = element.select(&title_selector).next();
        let desc_el = element.select(&desc_selector).next();

        if let Some(t_el) = title_el {
            let href = t_el.value().attr("href").unwrap_or("").to_string();
            let title_raw = t_el.text().collect::<Vec<_>>().join("");
            let title = title_raw.split_whitespace().collect::<Vec<_>>().join(" ");

            let summary = desc_el
                .map(|d| d.text().collect::<Vec<_>>().join(" ").trim().to_string())
                .unwrap_or_default();

            let title_lower = title.to_lowercase();
            let summary_lower = summary.to_lowercase();
            if title_lower.contains("ai")
                || title_lower.contains("llm")
                || title_lower.contains("gpt")
                || summary_lower.contains("ai")
                || summary_lower.contains("llm")
                || summary_lower.contains("gpt")
            {
                let clean_title = title.split('/').last().unwrap_or(&title).trim().to_string();
                let repository_url = format!("https://github.com{}", href);
                let thumbnail_url = github_preview_url(&repository_url);
                if let Some(preview_url) = &thumbnail_url {
                    eprintln!(
                        "[thumbnail][GitHub] item={clean_title:?} outcome=preview_url_generated image_url={preview_url}"
                    );
                } else {
                    eprintln!(
                        "[thumbnail][GitHub] item={clean_title:?} outcome=invalid_repository_url url={repository_url}"
                    );
                }
                items.push(DigestItem::new(
                    "GitHub",
                    Some("New releases"),
                    &clean_title,
                    &repository_url,
                    if summary.is_empty() {
                        None
                    } else {
                        Some(&summary)
                    },
                    thumbnail_url.as_deref(),
                    &today,
                ));
            }
        }
    }
    Ok(items)
}

pub async fn fetch_arxiv(client: &Client) -> Result<Vec<DigestItem>, String> {
    let url = "http://export.arxiv.org/api/query?search_query=cat:cs.AI&sortBy=submittedDate&sortOrder=descending&max_results=5";
    let bytes = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;
    let feed = feed_rs::parser::parse(bytes.as_ref()).map_err(|e| e.to_string())?;

    let mut items = Vec::new();
    for entry in feed.entries {
        let summary = entry.summary.map(|s| s.content).unwrap_or_default();
        let short_summary = if summary.len() > 200 {
            format!("{}...", &summary[..197])
        } else {
            summary
        };
        let mut item = DigestItem::new(
            "ArXiv",
            Some("AI"),
            &entry.title.map(|t| t.content).unwrap_or_default(),
            &entry
                .links
                .first()
                .map(|l| l.href.clone())
                .unwrap_or_default(),
            Some(&short_summary),
            None,
            &entry
                .published
                .or(entry.updated)
                .map(|d| d.to_rfc3339())
                .unwrap_or_default(),
        );
        // Abstracts do not have a meaningful thumbnail. Mark this as resolved so
        // the app never tries to fetch an image for the paper again.
        eprintln!(
            "[thumbnail][ArXiv] item={:?} outcome=source_fallback_no_image",
            item.title
        );
        item.mark_thumbnail_resolved();
        items.push(item);
    }
    Ok(items)
}

pub async fn fetch_rss(client: &Client) -> Result<Vec<DigestItem>, String> {
    let url = "https://techcrunch.com/category/artificial-intelligence/feed/";
    let bytes = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;
    let feed = feed_rs::parser::parse(bytes.as_ref()).map_err(|e| e.to_string())?;

    let thumbnail_client = image_client()?;
    let mut tasks = JoinSet::new();

    for (index, entry) in feed.entries.into_iter().take(8).enumerate() {
        let title = entry
            .title
            .as_ref()
            .map(|title| title.content.clone())
            .unwrap_or_default();
        let article_url = entry
            .links
            .first()
            .map(|link| link.href.clone())
            .unwrap_or_default();
        let raw_published_at = entry
            .published
            .or(entry.updated)
            .map(|date| date.to_rfc3339());
        let published_at = raw_published_at
            .as_deref()
            .and_then(|raw| parse_source_timestamp("RSS", raw));
        log_timestamp_sample(
            "RSS",
            raw_published_at.as_deref().unwrap_or("<missing>"),
            published_at.as_deref(),
            index,
        );
        let Some(published_at) = published_at else {
            eprintln!(
                "[timestamp][RSS] item={title:?} outcome=skipped_missing_or_invalid_timestamp"
            );
            continue;
        };
        let thumbnail_url = image_url_from_entry(&entry);
        let thumbnail_client = thumbnail_client.clone();

        tasks.spawn(async move {
            let thumbnail_url = match thumbnail_url {
                Some(thumbnail_url) => {
                    eprintln!(
                        "[thumbnail][RSS] item={title:?} outcome=feed_image image_url={thumbnail_url}"
                    );
                    Some(thumbnail_url)
                }
                None => resolve_og_image_for_item(
                    &thumbnail_client,
                    "RSS",
                    &title,
                    &article_url,
                )
                .await,
            };
            let mut item = DigestItem::new(
                "TechCrunch",
                Some("Tech"),
                &title,
                &article_url,
                None,
                thumbnail_url.as_deref(),
                &published_at,
            );
            item.mark_thumbnail_resolved();
            item
        });
    }

    let mut items = Vec::new();
    while let Some(result) = tasks.join_next().await {
        if let Ok(item) = result {
            items.push(item);
        }
    }

    Ok(items)
}
