//! MangaBaka Provider —— 对应 `snd.komf.providers.mangabaka` 包。
//!
//! 使用公开 API（`https://api.mangabaka.org`，无需密钥）：
//! - `GET /v1/series/search?q=...&type=...&type_not=...`
//! - `GET /v1/series/{id}`
//!
//! 对应 Kotlin 文件：`providers/mangabaka/MangaBakaMetadataProvider.kt`、
//! `MangaBakaMetadataMapper.kt`、`MangaBakaDataSource.kt`、`MangaBakaDbDataSource.kt`，
//! 及 `snd.komf.mangabaka` 包：`model/MangaBakaSeries.kt`（DTO 与枚举）、
//! `external/MangaBakaApiClient.kt`、`external/MangaBakaDbDownloader.kt`、
//! `repository/MangaBakaRepository.kt`。
//! 两种模式均实现：`mode: API`（默认，在线 API）与 `mode: DATABASE`（本地 SQLite，
//! 经 `update-mangabaka-db` 下载并建立 FTS5 索引）。
use crate::config::{MangaBakaConfig, SeriesMetadataConfig};
use crate::model::{
    Author, Image, MatchQuery, ProviderBookId, ProviderBookMetadata, ProviderSeriesId,
    ProviderSeriesMetadata, Publisher, PublisherType, ReleaseDate, SeriesBook, SeriesMetadata,
    SeriesSearchResult, SeriesStatus, SeriesTitle, TitleType, WebLink,
};
use crate::providers::{CoreProviders, MetadataProvider, ProviderError};
use crate::util::NameSimilarityMatcher;
use komf_api_models::config::DownloadProgress;
use rusqlite::OptionalExtension;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// API 模型（snake_case 与 MangaBaka API 一致）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct MangaBakaSearchResponse {
    pub data: Vec<MangaBakaSeriesDto>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MangaBakaSeriesResponse {
    pub data: MangaBakaSeriesDto,
}

/// `GET /v1/tags` 响应：`{ "data": [ ... ] }`（上游 `MangaBakaResponse<List<MangaBakaSeriesTag>>`）。
#[derive(Debug, Clone, Deserialize)]
pub struct MangaBakaTagsResponse {
    pub data: Vec<MangaBakaSeriesTagDto>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MangaBakaSeriesDto {
    pub id: i64,
    #[serde(default)]
    pub artists: Option<Vec<String>>,
    #[serde(default)]
    pub authors: Option<Vec<String>>,
    #[serde(default)]
    pub canonical_url: String,
    #[serde(default)]
    pub cover: MangaBakaCoverDto,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub final_volume: Option<String>,
    #[serde(default)]
    pub publishers: Option<Vec<MangaBakaPublisherDto>>,
    #[serde(default)]
    pub rating: Option<f64>,
    pub status: MangaBakaStatusDto,
    #[allow(dead_code)]
    r#type: MangaBakaTypeDto,
    #[serde(default)]
    pub links_v2: Option<Vec<MangaBakaLinkDto>>,
    #[serde(default)]
    pub published: Option<MangaBakaPublishedDateDto>,
    #[serde(default)]
    pub tags_v2: Option<Vec<MangaBakaSeriesTagDto>>,
    #[serde(default)]
    pub titles: Option<Vec<MangaBakaTitleDto>>,
    #[serde(default)]
    pub source: MangaBakaSourceDto,
    // ---- 管理 API 全字段（对应 Kotlin 领域模型；映射链路不消费的保持默认）----
    #[serde(default)]
    pub has_anime: bool,
    #[serde(default)]
    pub anime: Option<MangaBakaAnimeInfoDto>,
    #[serde(default)]
    pub content_rating: Option<MangaBakaContentRatingDto>,
    #[serde(default)]
    pub is_licensed: bool,
    #[serde(default)]
    pub last_updated_at: Option<String>,
    #[serde(default)]
    pub merged_with: Option<i64>,
    #[serde(default)]
    pub original_language: Option<String>,
    #[serde(default)]
    pub state: Option<MangaBakaSeriesStateDto>,
    #[serde(default)]
    pub total_chapters: Option<String>,
    #[serde(default)]
    pub relationships_v2: Option<Vec<MangaBakaRelationshipDto>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MangaBakaCoverDto {
    #[serde(default)]
    pub raw: Option<MangaBakaCoverRawDto>,
    #[serde(default)]
    pub x150: Option<MangaBakaCoverDpiDto>,
    #[serde(default)]
    pub x250: Option<MangaBakaCoverDpiDto>,
    #[serde(default)]
    pub x350: Option<MangaBakaCoverDpiDto>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MangaBakaCoverDpiDto {
    pub x1: Option<String>,
    #[allow(dead_code)]
    pub x2: Option<String>,
    #[allow(dead_code)]
    pub x3: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MangaBakaPublisherDto {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(rename = "type")]
    pub type_: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MangaBakaLinkDto {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    pub name_display: String,
    #[serde(default, rename = "type")]
    pub type_: Option<MangaBakaLinkTypeDto>,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MangaBakaPublishedDateDto {
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default)]
    pub end_date_is_estimated: Option<bool>,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub start_date_is_estimated: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MangaBakaSeriesTagDto {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub name_path: String,
    #[serde(default)]
    pub content_rating: Option<MangaBakaContentRatingDto>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub is_spoiler: Option<bool>,
    #[serde(default)]
    pub is_explicit: bool,
    #[serde(default)]
    pub implied_by_tag_ids: Vec<i64>,
    #[serde(default)]
    pub weight: Option<MangaBakaTagWeightDto>,
    #[serde(default)]
    pub level: i32,
    #[serde(default)]
    pub series_count: i32,
    #[serde(default)]
    pub is_genre: bool,
    #[serde(default)]
    pub parent_id: Option<i64>,
    #[serde(default)]
    pub merged_with: Option<i64>,
}

/// tags 表行 / `GET /v1/tags` 响应元素（管理 API 标签目录）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MangaBakaTagDto {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub parent_id: Option<i64>,
    #[serde(default)]
    pub merged_with: Option<i64>,
    pub name: String,
    #[serde(default)]
    pub name_path: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub is_spoiler: Option<bool>,
    #[serde(default)]
    pub is_genre: bool,
    pub content_rating: MangaBakaContentRatingDto,
    #[serde(default)]
    pub series_count: i32,
    #[serde(default)]
    pub level: i32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MangaBakaTitleDto {
    pub language: String,
    pub title: String,
    pub traits: Vec<String>,
    #[serde(default)]
    pub is_primary: Option<bool>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MangaBakaSourceDto {
    pub anilist: Option<MangaBakaSourceEntryDto>,
    pub anime_news_network: Option<MangaBakaSourceEntryDto>,
    pub anime_planet: Option<MangaBakaSourceEntryDto>,
    pub kitsu: Option<MangaBakaSourceEntryDto>,
    pub manga_updates: Option<MangaBakaSourceEntryDto>,
    pub my_anime_list: Option<MangaBakaSourceEntryDto>,
    pub shikimori: Option<MangaBakaSourceEntryDto>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MangaBakaSourceEntryDto {
    #[serde(default)]
    pub id: Option<serde_json::Value>,
    #[serde(default)]
    pub rating: Option<f64>,
    #[serde(default)]
    pub rating_normalized: Option<i32>,
}

impl MangaBakaSourceEntryDto {
    fn id_string(&self) -> Option<String> {
        self.id.as_ref().and_then(|v| match v {
            serde_json::Value::Number(n) => Some(n.to_string()),
            serde_json::Value::String(s) => Some(s.clone()),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MangaBakaStatusDto {
    Cancelled,
    Completed,
    Hiatus,
    Releasing,
    Upcoming,
    Unknown,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MangaBakaTypeDto {
    Manga,
    Novel,
    Manhwa,
    Manhua,
    Oel,
    Other,
}

impl MangaBakaTypeDto {
    fn as_str(&self) -> &'static str {
        match self {
            MangaBakaTypeDto::Manga => "manga",
            MangaBakaTypeDto::Novel => "novel",
            MangaBakaTypeDto::Manhwa => "manhwa",
            MangaBakaTypeDto::Manhua => "manhua",
            MangaBakaTypeDto::Oel => "oel",
            MangaBakaTypeDto::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MangaBakaContentRatingDto {
    Safe,
    Suggestive,
    Erotica,
    Pornographic,
}

impl Default for MangaBakaContentRatingDto {
    fn default() -> Self {
        MangaBakaContentRatingDto::Safe
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MangaBakaSeriesStateDto {
    Active,
    Merged,
    Deleted,
}

impl Default for MangaBakaSeriesStateDto {
    fn default() -> Self {
        MangaBakaSeriesStateDto::Active
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MangaBakaTitleTraitDto {
    Official,
    Native,
    Alternative,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MangaBakaLinkTypeDto {
    Publisher,
    Retailer,
    Webplatform,
    Info,
    Social,
    News,
    Piracy,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MangaBakaRelationTypeDto {
    Adaptation,
    Alternative,
    Cameo,
    CharacterFocus,
    Compilation,
    Contains,
    Crossover,
    Expansion,
    Main,
    Other,
    Parent,
    Parody,
    Prequel,
    Reboot,
    Remake,
    Sequel,
    Series,
    SideStory,
    Source,
    SpinOff,
    Summary,
    Uncollected,
}

impl Default for MangaBakaRelationTypeDto {
    fn default() -> Self {
        MangaBakaRelationTypeDto::Other
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MangaBakaRelationshipChronologyDto {
    Narrative,
    Release,
    Unknown,
}

impl Default for MangaBakaRelationshipChronologyDto {
    fn default() -> Self {
        MangaBakaRelationshipChronologyDto::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MangaBakaTagWeightDto {
    Core,
    Defining,
    Recurrent,
    Incidental,
    Unweighted,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MangaBakaAnimeInfoDto {
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub end: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MangaBakaCoverRawDto {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub size: Option<i64>,
    #[serde(default)]
    pub height: Option<i32>,
    #[serde(default)]
    pub width: Option<i32>,
    #[serde(default)]
    pub blurhash: Option<String>,
    #[serde(default)]
    pub thumbhash: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MangaBakaRelationshipDto {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub chronology: MangaBakaRelationshipChronologyDto,
    #[serde(default)]
    pub is_manual: bool,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub relation_type: MangaBakaRelationTypeDto,
    #[serde(default)]
    pub to_series_id: i64,
}

/// Komga 系列与其匹配的 MangaBaka 系列（`find` / `findAllLinked` 返回）。
#[derive(Debug, Clone)]
pub struct MangaBakaLinkedSeriesDto {
    pub komga_id: String,
    pub series: MangaBakaSeriesDto,
}

// ---------------------------------------------------------------------------
// API 客户端 —— 对应 `MangaBakaApiClient.kt`
// ---------------------------------------------------------------------------

const BASE_URL: &str = "https://api.mangabaka.org";

pub struct MangaBakaApiClient {
    pub http: reqwest::Client,
}

impl MangaBakaApiClient {
    fn new(http: reqwest::Client) -> Self {
        Self { http }
    }

    async fn search(
        &self,
        title: &str,
        types: &[MangaBakaTypeDto],
        types_not: &[MangaBakaTypeDto],
    ) -> Result<Vec<MangaBakaSeriesDto>, ProviderError> {
        let mut request = self
            .http
            .get(format!("{BASE_URL}/v1/series/search"))
            .query(&[("q", title)]);
        for t in types {
            request = request.query(&[("type", t.as_str())]);
        }
        for t in types_not {
            request = request.query(&[("type_not", t.as_str())]);
        }
        let response = request.send().await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::Status(
                CoreProviders::MangaBaka,
                status,
                body,
            ));
        }
        let parsed: MangaBakaSearchResponse = response.json().await?;
        Ok(parsed.data)
    }

    async fn get_series(&self, id: i64) -> Result<MangaBakaSeriesDto, ProviderError> {
        let response = self
            .http
            .get(format!("{BASE_URL}/v1/series/{id}"))
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::Status(
                CoreProviders::MangaBaka,
                status,
                body,
            ));
        }
        let parsed: MangaBakaSeriesResponse = response.json().await?;
        Ok(parsed.data)
    }
}

// ---------------------------------------------------------------------------
// Metadata mapper —— 对应 `MangaBakaMetadataMapper.kt`
// ---------------------------------------------------------------------------

pub struct MangaBakaMetadataMapper {
    pub metadata_config: SeriesMetadataConfig,
    pub author_roles: Vec<crate::model::AuthorRole>,
    pub artist_roles: Vec<crate::model::AuthorRole>,
    /// 全局 postProcessing.seriesTitleLanguage 联动：搜索/写入时按该语言选主标题（仅显示，不参与匹配）
    pub series_title_language: Option<String>,
}

impl MangaBakaMetadataMapper {
    pub fn new(
        metadata_config: SeriesMetadataConfig,
        author_roles: Vec<crate::model::AuthorRole>,
        artist_roles: Vec<crate::model::AuthorRole>,
        series_title_language: Option<String>,
    ) -> Self {
        Self {
            metadata_config,
            author_roles,
            artist_roles,
            series_title_language,
        }
    }

    fn to_series_metadata(
        &self,
        series: &MangaBakaSeriesDto,
        thumbnail: Option<Image>,
    ) -> ProviderSeriesMetadata {
        let cfg = &self.metadata_config;

        let status = Some(map_status(&series.status));
        let status_field = cfg.status.then_some(status).flatten();

        // 标题处理：原生（native 且非 -Latn）、罗马音、本地化
        let mut all_titles: Vec<&MangaBakaTitleDto> = series.titles.iter().flatten().collect();
        all_titles.sort_by(|a, b| {
            let pa = a.is_primary.unwrap_or(false);
            let pb = b.is_primary.unwrap_or(false);
            pb.cmp(&pa)
        });
        let native_title = all_titles
            .iter()
            .find(|t| t.traits.iter().any(|x| x == "native") && !t.language.ends_with("-Latn"));
        let titles: Vec<SeriesTitle> = all_titles
            .iter()
            .map(|t| {
                let romanized = t.title.ends_with("-Latn");
                let is_native = native_title == Some(t);
                SeriesTitle {
                    name: t.title.clone(),
                    r#type: Some(if is_native {
                        TitleType::Native
                    } else if romanized {
                        TitleType::Romaji
                    } else {
                        TitleType::Localized
                    }),
                    language: Some(t.language.replace("-Latn", "-ro")),
                }
            })
            .collect();
        // 写入侧不干涉标题语言：postProcessing.seriesTitleLanguage 由后处理阶段应用。
        let title_field = cfg.title.then(|| {
            titles.first().cloned().unwrap_or_else(|| SeriesTitle {
                name: String::new(),
                r#type: None,
                language: None,
            })
        });
        // Kotlin seriesTitles：title 配置关闭时保留标题列表，仅清空 type/language
        let titles_field = if cfg.title {
            titles.clone()
        } else {
            titles
                .iter()
                .map(|t| SeriesTitle {
                    r#type: None,
                    language: None,
                    ..t.clone()
                })
                .collect()
        };

        // 作者 / 艺术家
        let authors = if cfg.authors {
            let mut authors: Vec<Author> = series
                .authors
                .iter()
                .flatten()
                .flat_map(|name| {
                    self.author_roles.iter().map(|role| Author {
                        name: name.clone(),
                        role: *role,
                    })
                })
                .collect();
            authors.extend(series.artists.iter().flatten().flat_map(|name| {
                self.artist_roles.iter().map(|role| Author {
                    name: name.clone(),
                    role: *role,
                })
            }));
            authors
        } else {
            Vec::new()
        };

        // 出版社（Original / English）
        let mut original_publishers: Vec<Publisher> = series
            .publishers
            .iter()
            .flatten()
            .filter(|p| p.type_.as_deref() == Some("Original"))
            .filter_map(|p| p.name.clone())
            .map(|name| Publisher {
                name,
                r#type: Some(PublisherType::Original),
                language_tag: None,
            })
            .collect();
        dedup_publishers(&mut original_publishers);
        let mut english_publishers: Vec<Publisher> = series
            .publishers
            .iter()
            .flatten()
            .filter(|p| p.type_.as_deref() == Some("English"))
            .filter_map(|p| p.name.clone())
            .map(|name| Publisher {
                name,
                r#type: Some(PublisherType::Localized),
                language_tag: Some("en".to_string()),
            })
            .collect();
        dedup_publishers(&mut english_publishers);
        let publisher = if cfg.publisher {
            if cfg.use_original_publisher {
                original_publishers.first().cloned()
            } else {
                english_publishers
                    .first()
                    .cloned()
                    .or_else(|| original_publishers.first().cloned())
            }
        } else {
            None
        };
        let alternative_publishers: Vec<Publisher> = if cfg.publisher {
            original_publishers
                .iter()
                .chain(english_publishers.iter())
                .filter(|p| Some(*p) != publisher.as_ref())
                .cloned()
                .collect()
        } else {
            Vec::new()
        };

        // 链接：官方链接（links_v2，按 label 排序）+ 来源链接
        let official_links: Vec<WebLink> = if cfg.links {
            let mut links: Vec<WebLink> = series
                .links_v2
                .iter()
                .flatten()
                .filter_map(|link| {
                    official_link_url(&link.url).map(|url| WebLink {
                        label: link.name_display.clone(),
                        url,
                    })
                })
                .collect();
            links.sort_by(|a, b| a.label.cmp(&b.label));
            links
        } else {
            Vec::new()
        };
        let source_links: Vec<WebLink> = if cfg.links {
            let mut links = Vec::new();
            links.push(WebLink {
                label: "MangaBaka".to_string(),
                url: series.canonical_url.clone(),
            });
            let source = &series.source;
            if let Some(id) = source.anilist.as_ref().and_then(|s| s.id_string()) {
                links.push(WebLink {
                    label: "AniList".to_string(),
                    url: format!("https://anilist.co/manga/{id}"),
                });
            }
            if let Some(id) = source
                .anime_news_network
                .as_ref()
                .and_then(|s| s.id_string())
            {
                links.push(WebLink {
                    label: "AnimeNewsNetwork".to_string(),
                    url: format!("https://www.animenewsnetwork.com/encyclopedia/manga.php?id={id}"),
                });
            }
            if let Some(id) = source.anime_planet.as_ref().and_then(|s| s.id_string()) {
                links.push(WebLink {
                    label: "AnimePlanet".to_string(),
                    url: format!("https://www.anime-planet.com/manga/{id}"),
                });
            }
            if let Some(id) = source.kitsu.as_ref().and_then(|s| s.id_string()) {
                links.push(WebLink {
                    label: "Kitsu".to_string(),
                    url: format!("https://kitsu.app/manga/{id}"),
                });
            }
            if let Some(id) = source.manga_updates.as_ref().and_then(|s| s.id_string()) {
                links.push(WebLink {
                    label: "MangaUpdates".to_string(),
                    url: format!("https://www.mangaupdates.com/series/{id}"),
                });
            }
            if let Some(id) = source.shikimori.as_ref().and_then(|s| s.id_string()) {
                links.push(WebLink {
                    label: "Shikimori".to_string(),
                    url: format!("https://shikimori.one/mangas/{id}"),
                });
            }
            if let Some(id) = source.my_anime_list.as_ref().and_then(|s| s.id_string()) {
                links.push(WebLink {
                    label: "MyAnimeList".to_string(),
                    url: format!("https://myanimelist.net/manga/{id}"),
                });
            }
            links
        } else {
            Vec::new()
        };

        // 类型 / 标签
        let all_tags: Vec<&MangaBakaSeriesTagDto> = series.tags_v2.iter().flatten().collect();
        let genres = if cfg.genres {
            all_tags
                .iter()
                .filter(|t| t.is_genre)
                .map(|t| t.name.clone())
                .collect()
        } else {
            Vec::new()
        };
        let tags = if cfg.tags {
            all_tags
                .iter()
                .filter(|t| !t.is_genre)
                .map(|t| t.name.clone())
                .collect()
        } else {
            Vec::new()
        };

        let total_book_count = cfg
            .total_book_count
            .then(|| {
                series
                    .final_volume
                    .as_deref()
                    .and_then(|v| v.trim().parse::<i32>().ok())
            })
            .flatten();

        let summary = cfg
            .summary
            .then(|| series.description.as_ref().map(|d| html_to_text(d)))
            .flatten();

        let release_date = cfg
            .release_date
            .then(|| {
                series
                    .published
                    .as_ref()
                    .and_then(|p| p.start_date.as_ref())
                    .and_then(|d| parse_date_parts(d))
                    .map(|(year, month, day)| ReleaseDate::new(Some(year), month, day))
            })
            .flatten();

        let score = cfg.score.then_some(series.rating).flatten();

        let metadata = SeriesMetadata {
            status: status_field,
            title: title_field,
            titles: titles_field,
            summary,
            publisher,
            alternative_publishers,
            reading_direction: None,
            age_rating: None,
            language: None,
            genres,
            tags,
            total_book_count,
            authors,
            release_date,
            links: {
                let mut links = official_links;
                links.extend(source_links);
                links
            },
            score,
            thumbnail,
        };

        let books = if cfg.books {
            Vec::<SeriesBook>::new()
        } else {
            Vec::new()
        };

        ProviderSeriesMetadata {
            id: ProviderSeriesId(series.id.to_string()),
            metadata,
            books,
        }
    }

    fn to_series_search_result(&self, series: &MangaBakaSeriesDto) -> SeriesSearchResult {
        SeriesSearchResult {
            url: Some(format!("https://mangabaka.org/{}", series.id)),
            image_url: series.cover.x350.as_ref().and_then(|c| c.x1.clone()),
            title: primary_title(series, self.series_title_language.as_deref()),
            provider: CoreProviders::MangaBaka.as_str().to_string(),
            result_id: series.id.to_string(),
            media_type: None,
            language: None,
            nsfw: None,
        }
    }
}

/// 主标题 —— 对应 Kotlin `getPrimaryTitle`。
fn primary_title(series: &MangaBakaSeriesDto, preference: Option<&str>) -> String {
    let titles = match &series.titles {
        Some(titles) => titles,
        None => return String::new(),
    };
    // 标题语言偏好：命中偏好语言（前缀匹配，排除 -Latn 罗马音变体）；未命中回落 native→primary_en→first。
    if let Some(pref) = preference {
        if let Some(t) = titles
            .iter()
            .find(|t| t.language.starts_with(pref) && !t.language.ends_with("-Latn"))
        {
            return t.title.clone();
        }
    }
    if let Some(native) = titles
        .iter()
        .find(|t| t.traits.iter().any(|x| x == "native"))
    {
        return native.title.clone();
    }
    if let Some(primary_en) = titles
        .iter()
        .find(|t| t.language == "en" && t.is_primary == Some(true))
    {
        return primary_en.title.clone();
    }
    titles.first().map(|t| t.title.clone()).unwrap_or_default()
}

/// 状态映射 —— 对应 Kotlin `when (series.status)`。
fn map_status(status: &MangaBakaStatusDto) -> SeriesStatus {
    match status {
        MangaBakaStatusDto::Releasing
        | MangaBakaStatusDto::Upcoming
        | MangaBakaStatusDto::Unknown => SeriesStatus::Ongoing,
        MangaBakaStatusDto::Completed => SeriesStatus::Completed,
        MangaBakaStatusDto::Cancelled => SeriesStatus::Abandoned,
        MangaBakaStatusDto::Hiatus => SeriesStatus::Hiatus,
    }
}

/// 解析 `YYYY-MM-DD`。
fn parse_date_parts(value: &str) -> Option<(i32, Option<u32>, Option<u32>)> {
    let mut parts = value.split('-');
    let year = parts.next()?.parse().ok()?;
    let month = parts.next().and_then(|m| m.parse().ok());
    let day = parts.next().and_then(|d| d.parse().ok());
    Some((year, month, day))
}

/// 轻量 HTML → 纯文本（对应 Kotlin `Ksoup.parse(it).wholeText()`）。
fn html_to_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_entity = false;
    let mut entity = String::new();
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            '&' if !in_tag => {
                in_entity = true;
                entity.clear();
            }
            ';' if in_entity => {
                in_entity = false;
                text.push_str(&decode_entity(&entity));
                entity.clear();
            }
            _ if in_tag => {}
            _ if in_entity => entity.push(ch),
            _ => text.push(ch),
        }
    }
    if !entity.is_empty() {
        text.push_str(&decode_entity(&entity));
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_entity(entity: &str) -> String {
    match entity {
        "amp" => "&".to_string(),
        "lt" => "<".to_string(),
        "gt" => ">".to_string(),
        "quot" => "\"".to_string(),
        "apos" => "'".to_string(),
        "nbsp" => " ".to_string(),
        _ => format!("&{entity};"),
    }
}

/// 对应 Kotlin `parseUrl(link.url)?.toStingEncoded()`：校验 URL 可解析，重建并
/// 百分号编码 path/query/fragment（已有 %XX 保留，不双重编码）。
fn official_link_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let mut out = String::new();
    out.push_str(parsed.scheme());
    out.push_str("://");
    out.push_str(parsed.host_str()?);
    if let Some(port) = parsed.port() {
        out.push_str(&format!(":{port}"));
    }
    let path = parsed.path();
    // url crate 会把无 path 的 URL 规范化为 "/"，需用原始字符串区分是否有 path 分隔符
    let mut authority = String::new();
    authority.push_str(parsed.host_str()?);
    if let Some(port) = parsed.port() {
        authority.push_str(&format!(":{port}"));
    }
    let prefix_len = parsed.scheme().len() + 3 + authority.len();
    let has_path_slash = url
        .as_bytes()
        .get(prefix_len)
        .map(|b| *b == b'/')
        .unwrap_or(false);
    if path == "/" && has_path_slash {
        out.push('/');
    } else if path != "/" && !path.is_empty() {
        let encoded = path
            .trim_start_matches('/')
            .split('/')
            .map(|seg| encode_url_component(seg, is_path_segment_keep))
            .collect::<Vec<_>>()
            .join("/");
        out.push('/');
        out.push_str(&encoded);
    }
    if let Some(query) = parsed.query() {
        if !query.is_empty() {
            out.push('?');
            for (i, pair) in query.split('&').enumerate() {
                if i > 0 {
                    out.push('&');
                }
                match pair.split_once('=') {
                    Some((k, v)) => {
                        out.push_str(&encode_url_component(k, is_query_keep));
                        out.push('=');
                        out.push_str(&encode_url_component(v, is_query_keep));
                    }
                    None => out.push_str(&encode_url_component(pair, is_query_keep)),
                }
            }
        }
    }
    if let Some(fragment) = parsed.fragment() {
        if !fragment.is_empty() {
            out.push('#');
            out.push_str(&encode_url_component(fragment, is_fragment_keep));
        }
    }
    Some(out)
}

/// 百分号编码单个组件：保留字符与已有 %XX 原样保留，其余按 UTF-8 编码。
fn encode_url_component(s: &str, keep: fn(char) -> bool) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            out.push_str(&s[i..i + 3]);
            i += 3;
            continue;
        }
        let ch = s[i..].chars().next().unwrap();
        if keep(ch) {
            out.push(ch);
        } else {
            let mut buf = [0u8; 4];
            for b in ch.encode_utf8(&mut buf).as_bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        i += ch.len_utf8();
    }
    out
}

/// path 段保留集：unreserved + 子分隔符 + `:` `@`（对应 Kotlin encodeURLPath 默认）。
fn is_path_segment_keep(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || "-._~!$&'()*+,;=:@".contains(ch)
}

/// query key/value 保留集：unreserved + `!$'()*+,;=:@`（对应 Kotlin encodeURLQueryComponent；
/// `&` 为分隔符、`=` 为 key/value 分隔符，二者由调用方处理）。
fn is_query_keep(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || "-._~!$'()*+,;=:@".contains(ch)
}

/// fragment 保留集：unreserved + `!$&'()*+,;=:@`（对应 Kotlin encodeURLParameter）。
fn is_fragment_keep(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || "-._~!$&'()*+,;=:@".contains(ch)
}

// ---------------------------------------------------------------------------
// Provider —— 对应 `MangaBakaMetadataProvider.kt`
// ---------------------------------------------------------------------------
// 简单 TTL 缓存 —— 对应 Kotlin cache4k expireAfterWrite(30.minutes)。
// Kotlin 未配置 maximumSize（cache4k 默认无条目上限），此处保持一致。
// ---------------------------------------------------------------------------

pub struct TtlCache<K, V> {
    inner: tokio::sync::Mutex<HashMap<K, (V, Instant)>>,
    ttl: Duration,
}

impl<K, V> TtlCache<K, V>
where
    K: Eq + std::hash::Hash + Clone,
    V: Clone,
{
    fn new(ttl: Duration) -> Self {
        Self {
            inner: tokio::sync::Mutex::new(HashMap::new()),
            ttl,
        }
    }

    /// Kotlin cache4k：命中且未过期直接返回；未命中执行 load，仅成功结果入缓存。
    async fn get_or_load<E, F, Fut>(&self, key: K, load: F) -> Result<V, E>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<V, E>>,
    {
        {
            let guard = self.inner.lock().await;
            if let Some((value, created)) = guard.get(&key) {
                if created.elapsed() < self.ttl {
                    return Ok(value.clone());
                }
            }
        }
        let value = load().await?;
        let mut guard = self.inner.lock().await;
        guard.insert(key, (value.clone(), Instant::now()));
        Ok(value)
    }

    async fn put(&self, key: K, value: V) {
        let mut guard = self.inner.lock().await;
        guard.insert(key, (value, Instant::now()));
    }
}

// ---------------------------------------------------------------------------

pub struct MangaBakaMetadataProvider {
    pub data_source: MangaBakaDataSource,
    pub metadata_mapper: MangaBakaMetadataMapper,
    pub name_matcher: NameSimilarityMatcher,
    pub cover_fetch_client: Option<reqwest::Client>,
    pub type_includes: Vec<MangaBakaTypeDto>,
    pub type_excludes: Vec<MangaBakaTypeDto>,
    /// 对应 Kotlin cache4k expireAfterWrite(30.minutes)（无 maximumSize）
    pub cache: TtlCache<i64, MangaBakaSeriesDto>,
}

pub fn create_provider(
    config: &MangaBakaConfig,
    default_name_matcher: NameSimilarityMatcher,
    http_client: &reqwest::Client,
    database_file: Option<&std::path::Path>,
    series_title_language: Option<String>,
) -> Option<MangaBakaMetadataProvider> {
    if !config.enabled {
        return None;
    }
    let data_source = match config.mode {
        crate::config::MangaBakaMode::Api => {
            MangaBakaDataSource::Api(MangaBakaApiClient::new(http_client.clone()))
        }
        crate::config::MangaBakaMode::Database => {
            match database_file {
                Some(path) if path.exists() => {
                    MangaBakaDataSource::Db(MangaBakaDbRepository::new(path.to_path_buf()))
                }
                _ => {
                    // 对应 Kotlin："Failed to find MangaBaka database. Disabling MangaBaka provider"
                    tracing::warn!(
                        "Failed to find MangaBaka database. Disabling MangaBaka provider"
                    );
                    return None;
                }
            }
        }
    };
    let type_excludes: Vec<MangaBakaTypeDto> = match config.media_type {
        crate::model::MediaType::Manga => vec![MangaBakaTypeDto::Novel],
        _ => Vec::new(),
    };
    let type_includes: Vec<MangaBakaTypeDto> = match config.media_type {
        crate::model::MediaType::Manga => Vec::new(),
        crate::model::MediaType::Novel => vec![MangaBakaTypeDto::Novel],
        crate::model::MediaType::Comic => vec![MangaBakaTypeDto::Oel, MangaBakaTypeDto::Other],
        crate::model::MediaType::Webtoon => {
            vec![MangaBakaTypeDto::Manhua, MangaBakaTypeDto::Manhwa]
        }
    };

    let name_matcher = config.name_matching_mode.unwrap_or(default_name_matcher);
    Some(MangaBakaMetadataProvider {
        data_source,
        metadata_mapper: MangaBakaMetadataMapper::new(
            config.series_metadata.clone(),
            config.author_roles.clone(),
            config.artist_roles.clone(),
            series_title_language,
        ),
        name_matcher,
        cover_fetch_client: config
            .series_metadata
            .thumbnail
            .then(|| http_client.clone()),
        type_includes,
        type_excludes,
        cache: TtlCache::new(Duration::from_secs(30 * 60)),
    })
}

impl MangaBakaMetadataProvider {
    async fn fetch_cover(&self, series: &MangaBakaSeriesDto) -> Option<Image> {
        let client = self.cover_fetch_client.as_ref()?;
        let url = series.cover.x350.as_ref()?.x1.as_ref()?;
        let response = client.get(url).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        // Kotlin：mime 取自响应 Content-Type
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .map(|ct| ct.to_str().unwrap_or("").to_string());
        let bytes = response.bytes().await.ok()?.to_vec();
        Some(Image::new(bytes, mime))
    }
}

/// 对应 Kotlin `.toSet()`：按 (name,type,languageTag) 去重，保留首次出现顺序。
fn dedup_publishers(v: &mut Vec<Publisher>) {
    let mut out: Vec<Publisher> = Vec::with_capacity(v.len());
    for p in v.drain(..) {
        if !out.iter().any(|e| e == &p) {
            out.push(p);
        }
    }
    *v = out;
}

#[async_trait::async_trait]
impl MetadataProvider for MangaBakaMetadataProvider {

    fn resolve_link_id(&self, query: &str) -> Option<String> {
        let re = regex::Regex::new(r"mangabaka\.org/(\d+)").ok()?;
        re.captures(query)
            .map(|c| c.get(1).unwrap().as_str().to_string())
    }
    fn provider_name(&self) -> CoreProviders {
        CoreProviders::MangaBaka
    }

    fn alternative_titles_enabled(&self) -> bool {
        self.metadata_mapper.metadata_config.alternative_titles
    }

    async fn resolve_link_search_result(&self, query: &str) -> Option<SeriesSearchResult> {
        let id: i64 = self.resolve_link_id(query)?.parse().ok()?;
        let series = self.data_source.get_series(id).await.ok()?;
        self.cache.put(id, series.clone()).await;
        Some(self.metadata_mapper.to_series_search_result(&series))
    }

    async fn get_series_metadata(
        &self,
        series_id: &ProviderSeriesId,
    ) -> Result<ProviderSeriesMetadata, ProviderError> {
        let id: i64 = series_id.0.parse().map_err(|_| {
            ProviderError::message(format!("invalid MangaBaka series id: {}", series_id.0))
        })?;
        let series = self.cache.get_or_load(id, || self.data_source.get_series(id)).await?;
        let cover = self.fetch_cover(&series).await;
        Ok(self.metadata_mapper.to_series_metadata(&series, cover))
    }

    async fn get_series_cover(
        &self,
        series_id: &ProviderSeriesId,
    ) -> Result<Option<Image>, ProviderError> {
        let id: i64 = series_id.0.parse().map_err(|_| {
            ProviderError::message(format!("invalid MangaBaka series id: {}", series_id.0))
        })?;
        let series = self.cache.get_or_load(id, || self.data_source.get_series(id)).await?;
        Ok(self.fetch_cover(&series).await)
    }

    async fn get_book_metadata(
        &self,
        _series_id: &ProviderSeriesId,
        _book_id: &ProviderBookId,
    ) -> Result<ProviderBookMetadata, ProviderError> {
        // 对应 Kotlin `TODO("Not yet implemented")`
        Err(ProviderError::NotImplemented(CoreProviders::MangaBaka))
    }

    async fn search_series(
        &self,
        series_name: &str,
        limit: usize,
        _media_type: Option<crate::model::MediaType>,
    ) -> Result<Vec<SeriesSearchResult>, ProviderError> {
        let results = self
            .data_source
            .search(series_name, &self.type_includes, &self.type_excludes)
            .await?;
        for r in &results {
            self.cache.put(r.id, r.clone()).await;
        }
        Ok(results
            .iter()
            .take(limit)
            .map(|r| self.metadata_mapper.to_series_search_result(r))
            .collect())
    }

    async fn match_series_metadata(
        &self,
        match_query: &MatchQuery,
    ) -> Result<Option<ProviderSeriesMetadata>, ProviderError> {
        let series_name = match_query.series_name.clone();
        let results = self
            .data_source
            .search(
                &series_name.chars().take(400).collect::<String>(),
                &self.type_includes,
                &self.type_excludes,
            )
            .await?;
        for r in &results {
            self.cache.put(r.id, r.clone()).await;
        }
        let matched = results.iter().find(|series| {
            let titles: Vec<String> = series
                .titles
                .as_ref()
                .map(|titles| titles.iter().map(|t| t.title.clone()).collect())
                .unwrap_or_default();
            self.name_matcher.matches(
                &match_query.normalized_series_name(),
                &match_query.normalize_titles(&titles),
            )
        });
        match matched {
            Some(series) => {
                let cover = self.fetch_cover(series).await;
                Ok(Some(self.metadata_mapper.to_series_metadata(series, cover)))
            }
            None => Ok(None),
        }
    }
}

// ---------------------------------------------------------------------------
// 数据源抽象 + DATABASE 模式 —— 对应 `MangaBakaDataSource.kt` / `db/*.kt`
// ---------------------------------------------------------------------------

/// 数据源：API（在线）或本地 SQLite 数据库（`MangaBakaMode.DATABASE`）。
pub enum MangaBakaDataSource {
    Api(MangaBakaApiClient),
    Db(MangaBakaDbRepository),
}

impl MangaBakaDataSource {
    async fn search(
        &self,
        title: &str,
        types: &[MangaBakaTypeDto],
        types_not: &[MangaBakaTypeDto],
    ) -> Result<Vec<MangaBakaSeriesDto>, ProviderError> {
        match self {
            MangaBakaDataSource::Api(client) => client.search(title, types, types_not).await,
            MangaBakaDataSource::Db(repo) => repo.search(title, types, types_not),
        }
    }

    async fn get_series(&self, id: i64) -> Result<MangaBakaSeriesDto, ProviderError> {
        match self {
            MangaBakaDataSource::Api(client) => client.get_series(id).await,
            MangaBakaDataSource::Db(repo) => repo.get_series(id),
        }
    }
}

// ---------------------------------------------------------------------------
// DATABASE 数据访问 —— 对应 `MangaBakaDbDataSource.kt` / `repository/MangaBakaRepository.kt`
// ---------------------------------------------------------------------------

/// 本地 SQLite 只读仓储。连接按查询打开（保证 `Sync`，行为与 Kotlin 每次 transaction 等价）。
pub struct MangaBakaDbRepository {
    pub database_file: PathBuf,
}

const MANGA_BAKA_DB_URL: &str = "https://api.mangabaka.org/v1/database/series.sqlite.tar.gz";
const MANGA_BAKA_CHECKSUM_URL: &str =
    "https://api.mangabaka.org/v1/database/series.sqlite.tar.gz.sha1";

impl MangaBakaDbRepository {
    pub fn new(database_file: impl Into<PathBuf>) -> Self {
        Self {
            database_file: database_file.into(),
        }
    }

    fn open(&self) -> Result<rusqlite::Connection, ProviderError> {
        let conn = rusqlite::Connection::open(&self.database_file)
            .map_err(|e| ProviderError::message(format!("failed to open MangaBaka database: {e}")))?;
        // 自建表（下载器保留 komga_series；tags/series_tags 在下载重建时由下载器重建，
        // 这里 ensure 是为了手工放置的库也能用管理 API）。
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS komga_series (
                komga_id TEXT NOT NULL,
                mangabaka_id INTEGER NOT NULL,
                PRIMARY KEY (komga_id, mangabaka_id)
             );
             CREATE INDEX IF NOT EXISTS idx_komga_series_mangabaka ON komga_series(mangabaka_id);
             CREATE TABLE IF NOT EXISTS tags (
                id INTEGER PRIMARY KEY,
                parent_id INTEGER,
                merged_with INTEGER,
                name TEXT NOT NULL,
                name_path TEXT NOT NULL DEFAULT '',
                description TEXT,
                is_spoiler INTEGER NOT NULL DEFAULT 0,
                is_genre INTEGER NOT NULL DEFAULT 0,
                content_rating TEXT NOT NULL,
                series_count INTEGER NOT NULL DEFAULT 0,
                level INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS series_tags (
                series_id INTEGER NOT NULL,
                tag_id INTEGER NOT NULL,
                is_spoiler INTEGER NOT NULL DEFAULT 0,
                is_explicit INTEGER NOT NULL DEFAULT 0,
                implied_by_tag_ids TEXT NOT NULL DEFAULT '[]',
                weight TEXT NOT NULL DEFAULT 'unweighted',
                PRIMARY KEY (tag_id, series_id)
             );",
        )
        .map_err(|e| ProviderError::message(format!("failed to init MangaBaka db tables: {e}")))?;
        Ok(conn)
    }

    /// 对应 `MangaBakaDbDataSource.search`：FTS5 titles MATCH + type IN/NOT IN + rank 排序。
    pub fn search(
        &self,
        title: &str,
        types: &[MangaBakaTypeDto],
        types_not: &[MangaBakaTypeDto],
    ) -> Result<Vec<MangaBakaSeriesDto>, ProviderError> {
        let conn = self.open()?;
        let quoted = format!("\"{title}\"");
        // 对应上游 `MangaBakaRepository.search`：titles_fts 每标题一行，标题级 rank 排序。
        let mut sql = String::from("SELECT id FROM titles_fts WHERE title MATCH ?");
        let mut params: Vec<rusqlite::types::Value> = vec![rusqlite::types::Value::Text(quoted)];
        if !types.is_empty() {
            let list = types.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            sql.push_str(&format!(" AND type IN ({list})"));
            params.extend(
                types
                    .iter()
                    .map(|t| rusqlite::types::Value::Text(t.as_str().to_string())),
            );
        }
        if !types_not.is_empty() {
            let list = types_not.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            sql.push_str(&format!(" AND type NOT IN ({list})"));
            params.extend(
                types_not
                    .iter()
                    .map(|t| rusqlite::types::Value::Text(t.as_str().to_string())),
            );
        }
        sql.push_str(" ORDER BY rank LIMIT 24");

        let ids: Vec<i64> = {
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| ProviderError::message(format!("MangaBaka db search prepare: {e}")))?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(params.iter()), |row| row.get(0))
                .map_err(|e| ProviderError::message(format!("MangaBaka db search: {e}")))?;
            let ids: Vec<i64> = rows
                .collect::<Result<_, _>>()
                .map_err(|e| ProviderError::message(format!("MangaBaka db search rows: {e}")))?;
            drop(stmt);
            ids
        };

        if ids.is_empty() {
            return Ok(Vec::new());
        }
        self.fetch_series_by_ids(&conn, &ids)
    }

    /// 对应 `MangaBakaDbDataSource.getSeries`：按 id 查询，无结果抛错。
    pub fn get_series(&self, id: i64) -> Result<MangaBakaSeriesDto, ProviderError> {
        let conn = self.open()?;
        self.fetch_series_by_ids(&conn, &[id])?
            .into_iter()
            .next()
            .ok_or_else(|| ProviderError::message(format!("failed to find series with id {id}")))
    }

    fn fetch_series_by_ids(
        &self,
        conn: &rusqlite::Connection,
        ids: &[i64],
    ) -> Result<Vec<MangaBakaSeriesDto>, ProviderError> {
        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!("SELECT * FROM series WHERE id IN ({placeholders})");
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| ProviderError::message(format!("MangaBaka db prepare: {e}")))?;
        let mut rows = {
            let mut rows = stmt
                .query(rusqlite::params_from_iter(ids.iter().map(|i| *i)))
                .map_err(|e| ProviderError::message(format!("MangaBaka db query: {e}")))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|e| ProviderError::message(format!("MangaBaka db rows: {e}")))?
            {
                out.push(row_to_series_dto(row)?);
            }
            out
        };
        drop(stmt);

        // 用 series_tags join tags 补全系列标签全字段（下载库存在时；映射链路只消费 name/is_genre）。
        if let Ok(tags_map) = self.select_full_tags(conn, ids) {
            for series in rows.iter_mut() {
                if let Some(full) = tags_map.get(&series.id) {
                    if !full.is_empty() {
                        series.tags_v2 = Some(full.clone());
                    }
                }
            }
        }
        Ok(rows)
    }

    /// `link`：校验系列存在后写入（先删除该 Komga 系列的旧关联再插入）。
    pub fn link(&self, komga_id: &str, mangabaka_id: i64) -> Result<(), ProviderError> {
        let conn = self.open()?;
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM series WHERE id = ?1",
                [mangabaka_id],
                |row| row.get(0),
            )
            .map_err(|e| ProviderError::message(format!("MangaBaka link check: {e}")))?;
        if exists == 0 {
            return Err(ProviderError::message(format!(
                "Series with id {mangabaka_id} does not exist"
            )));
        }
        conn.execute("DELETE FROM komga_series WHERE komga_id = ?1", [komga_id])
            .map_err(|e| ProviderError::message(format!("MangaBaka link delete: {e}")))?;
        conn.execute(
            "INSERT INTO komga_series (komga_id, mangabaka_id) VALUES (?1, ?2)",
            rusqlite::params![komga_id, mangabaka_id],
        )
        .map_err(|e| ProviderError::message(format!("MangaBaka link insert: {e}")))?;
        Ok(())
    }

    /// `unlink`：删除该 Komga 系列的关联。
    pub fn unlink(&self, komga_id: &str) -> Result<(), ProviderError> {
        let conn = self.open()?;
        conn.execute("DELETE FROM komga_series WHERE komga_id = ?1", [komga_id])
            .map_err(|e| ProviderError::message(format!("MangaBaka unlink: {e}")))?;
        Ok(())
    }

    /// `find`：按 Komga 系列 id 查关联。
    pub fn find(&self, komga_id: &str) -> Result<Option<MangaBakaLinkedSeriesDto>, ProviderError> {
        let conn = self.open()?;
        let mangabaka_id: Option<i64> = conn
            .query_row(
                "SELECT mangabaka_id FROM komga_series WHERE komga_id = ?1",
                [komga_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| ProviderError::message(format!("MangaBaka find: {e}")))?;
        let Some(mangabaka_id) = mangabaka_id else {
            return Ok(None);
        };
        let series = self.fetch_series_by_ids(&conn, &[mangabaka_id])?;
        Ok(series.into_iter().next().map(|series| MangaBakaLinkedSeriesDto {
            komga_id: komga_id.to_string(),
            series,
        }))
    }

    /// `findAllLinked`：批量查询 Komga 系列关联（保持输入顺序）。
    pub fn find_all_linked(&self, komga_ids: &[String]) -> Result<Vec<MangaBakaLinkedSeriesDto>, ProviderError> {
        if komga_ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.open()?;
        let placeholders = komga_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT komga_id, mangabaka_id FROM komga_series WHERE komga_id IN ({placeholders})"
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| ProviderError::message(format!("MangaBaka findAllLinked prepare: {e}")))?;
        let pairs: Vec<(String, i64)> = {
            let mut rows = stmt
                .query(rusqlite::params_from_iter(komga_ids.iter()))
                .map_err(|e| ProviderError::message(format!("MangaBaka findAllLinked: {e}")))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|e| ProviderError::message(format!("MangaBaka findAllLinked rows: {e}")))?
            {
                out.push((row.get(0)?, row.get(1)?));
            }
            out
        };
        drop(stmt);

        let mangabaka_ids: Vec<i64> = pairs.iter().map(|(_, id)| *id).collect();
        let series = self.fetch_series_by_ids(&conn, &mangabaka_ids)?;
        let by_id: std::collections::HashMap<i64, &MangaBakaSeriesDto> =
            series.iter().map(|s| (s.id, s)).collect();
        let mut out = Vec::new();
        for (komga_id, mangabaka_id) in pairs {
            if let Some(series) = by_id.get(&mangabaka_id) {
                out.push(MangaBakaLinkedSeriesDto {
                    komga_id,
                    series: (*series).clone(),
                });
            }
        }
        Ok(out)
    }

    /// `getAllTags`：返回标签目录（tags 表；未下载/无 tags 表时返回空）。
    pub fn get_all_tags(&self) -> Result<Vec<MangaBakaTagDto>, ProviderError> {
        let conn = self.open()?;
        let mut stmt = conn
            .prepare("SELECT id, parent_id, merged_with, name, name_path, description, is_spoiler, is_genre, content_rating, series_count, level FROM tags")
            .map_err(|e| ProviderError::message(format!("MangaBaka getTags prepare: {e}")))?;
        let tags = stmt
            .query_map([], |row| {
                Ok(MangaBakaTagDto {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    merged_with: row.get(2)?,
                    name: row.get(3)?,
                    name_path: row.get(4)?,
                    description: row.get(5)?,
                    is_spoiler: row.get::<_, Option<i64>>(6)?.map(|v| v != 0),
                    is_genre: row.get::<_, i64>(7)? != 0,
                    content_rating: parse_content_rating_db(&row.get::<_, Option<String>>(8)?),
                    series_count: row.get(9)?,
                    level: row.get(10)?,
                })
            })
            .map_err(|e| ProviderError::message(format!("MangaBaka getTags query: {e}")))?;
        tags.collect::<Result<Vec<_>, _>>()
            .map_err(|e| ProviderError::message(format!("MangaBaka getTags rows: {e}")))
    }

    /// series_tags join tags：按 series_id 组装完整系列标签（含 name/content_rating 等目录字段）。
    fn select_full_tags(
        &self,
        conn: &rusqlite::Connection,
        ids: &[i64],
    ) -> Result<std::collections::HashMap<i64, Vec<MangaBakaSeriesTagDto>>, ProviderError> {
        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT st.series_id, st.tag_id, st.is_spoiler, st.is_explicit, st.implied_by_tag_ids, st.weight,              t.parent_id, t.merged_with, t.name, t.name_path, t.description, t.is_genre, t.content_rating,              t.series_count, t.level              FROM series_tags st LEFT JOIN tags t ON t.id = st.tag_id              WHERE st.series_id IN ({placeholders})"
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| ProviderError::message(format!("MangaBaka tags join prepare: {e}")))?;
        let mut out: std::collections::HashMap<i64, Vec<MangaBakaSeriesTagDto>> =
            std::collections::HashMap::new();
        let mut rows = stmt
            .query(rusqlite::params_from_iter(ids.iter().map(|i| *i)))
            .map_err(|e| ProviderError::message(format!("MangaBaka tags join query: {e}")))?;
        while let Some(row) = rows
            .next()
            .map_err(|e| ProviderError::message(format!("MangaBaka tags join rows: {e}")))?
        {
            let series_id: i64 = row.get(0)?;
            let implied: Vec<i64> = row
                .get::<_, Option<String>>(4)?
                .and_then(|v| serde_json::from_str(&v).ok())
                .unwrap_or_default();
            let weight: Option<MangaBakaTagWeightDto> = row
                .get::<_, Option<String>>(5)?
                .and_then(|v| parse_tag_weight(&v));
            let content_rating = parse_content_rating_db(&row.get::<_, Option<String>>(12)?);
            out.entry(series_id).or_default().push(MangaBakaSeriesTagDto {
                id: row.get(1)?,
                is_spoiler: row.get::<_, Option<i64>>(2)?.map(|v| v != 0),
                is_explicit: row.get::<_, i64>(3)? != 0,
                implied_by_tag_ids: implied,
                weight,
                parent_id: row.get(6)?,
                merged_with: row.get(7)?,
                name: row.get(8)?,
                name_path: row.get(9)?,
                description: row.get(10)?,
                is_genre: row.get::<_, i64>(11)? != 0,
                content_rating: Some(content_rating),
                series_count: row.get(13)?,
                level: row.get(14)?,
            });
        }
        Ok(out)
    }
}

fn parse_content_rating_db(value: &Option<String>) -> MangaBakaContentRatingDto {
    match value.as_deref().unwrap_or_default().to_lowercase().as_str() {
        "suggestive" => MangaBakaContentRatingDto::Suggestive,
        "erotica" => MangaBakaContentRatingDto::Erotica,
        "pornographic" => MangaBakaContentRatingDto::Pornographic,
        _ => MangaBakaContentRatingDto::Safe,
    }
}

fn parse_tag_weight(value: &str) -> Option<MangaBakaTagWeightDto> {
    match value.to_lowercase().as_str() {
        "core" => Some(MangaBakaTagWeightDto::Core),
        "defining" => Some(MangaBakaTagWeightDto::Defining),
        "recurrent" => Some(MangaBakaTagWeightDto::Recurrent),
        "incidental" => Some(MangaBakaTagWeightDto::Incidental),
        "unweighted" => Some(MangaBakaTagWeightDto::Unweighted),
        _ => None,
    }
}

impl MangaBakaContentRatingDto {
    fn as_db_str(&self) -> &'static str {
        match self {
            MangaBakaContentRatingDto::Safe => "safe",
            MangaBakaContentRatingDto::Suggestive => "suggestive",
            MangaBakaContentRatingDto::Erotica => "erotica",
            MangaBakaContentRatingDto::Pornographic => "pornographic",
        }
    }
}

/// 从 SQLite 行构造 API DTO —— 对应 Kotlin `ResultRow.toModel()`。
///
/// 仅填充映射链路消费的字段，其余保持 None/默认（与 Kotlin 领域模型到 API DTO
/// 的字段投影等价：API 响应多余字段不影响映射）。
fn row_to_series_dto(row: &rusqlite::Row<'_>) -> Result<MangaBakaSeriesDto, ProviderError> {
    let json_vec =
        |row: &rusqlite::Row<'_>, col: &str| -> Result<Option<serde_json::Value>, ProviderError> {
            match row.get::<_, Option<String>>(col)? {
                Some(text) => serde_json::from_str(&text)
                    .map(Some)
                    .map_err(|e| ProviderError::message(format!("MangaBaka db json {col}: {e}"))),
                None => Ok(None),
            }
        };
    let text = |row: &rusqlite::Row<'_>, col: &str| -> Result<Option<String>, ProviderError> {
        row.get(col)
            .map_err(|e| ProviderError::message(format!("MangaBaka db col {col}: {e}")))
    };

    let status = match text(row, "status")?
        .unwrap_or_default()
        .to_uppercase()
        .as_str()
    {
        "CANCELLED" => MangaBakaStatusDto::Cancelled,
        "COMPLETED" => MangaBakaStatusDto::Completed,
        "HIATUS" => MangaBakaStatusDto::Hiatus,
        "RELEASING" => MangaBakaStatusDto::Releasing,
        "UPCOMING" => MangaBakaStatusDto::Upcoming,
        _ => MangaBakaStatusDto::Unknown,
    };
    let type_ = match text(row, "type")?
        .unwrap_or_default()
        .to_uppercase()
        .as_str()
    {
        "MANGA" => MangaBakaTypeDto::Manga,
        "NOVEL" => MangaBakaTypeDto::Novel,
        "MANHWA" => MangaBakaTypeDto::Manhwa,
        "MANHUA" => MangaBakaTypeDto::Manhua,
        "OEL" => MangaBakaTypeDto::Oel,
        _ => MangaBakaTypeDto::Other,
    };
    let content_rating = match text(row, "content_rating")?
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "safe" => MangaBakaContentRatingDto::Safe,
        "suggestive" => MangaBakaContentRatingDto::Suggestive,
        "erotica" => MangaBakaContentRatingDto::Erotica,
        "pornographic" => MangaBakaContentRatingDto::Pornographic,
        _ => MangaBakaContentRatingDto::Safe,
    };
    let state = match text(row, "state")?
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "active" => MangaBakaSeriesStateDto::Active,
        "merged" => MangaBakaSeriesStateDto::Merged,
        "deleted" => MangaBakaSeriesStateDto::Deleted,
        _ => MangaBakaSeriesStateDto::Active,
    };

    let cover_x350 = text(row, "cover_x350_x1")?;
    let raw_cover = text(row, "cover_raw_url")?.map(|url| {
        MangaBakaCoverRawDto {
            url: Some(url),
            size: text(row, "cover_raw_size")
                .ok()
                .flatten()
                .and_then(|v| v.parse().ok()),
            height: text(row, "cover_raw_height")
                .ok()
                .flatten()
                .and_then(|v| v.parse().ok()),
            width: text(row, "cover_raw_width")
                .ok()
                .flatten()
                .and_then(|v| v.parse().ok()),
            blurhash: text(row, "cover_raw_blurhash").ok().flatten(),
            thumbhash: text(row, "cover_raw_thumbhash").ok().flatten(),
            format: text(row, "cover_raw_format").ok().flatten(),
        }
    });
    let cover = MangaBakaCoverDto {
        raw: raw_cover,
        x150: None,
        x250: None,
        x350: cover_x350.map(|url| MangaBakaCoverDpiDto {
            x1: Some(url),
            x2: None,
            x3: None,
        }),
    };

    let rating_col = |row: &rusqlite::Row<'_>, base: &str| -> Result<(Option<f64>, Option<i32>), ProviderError> {
        let rating = text(row, &format!("{base}_rating"))?.and_then(|v| v.parse().ok());
        let normalized = text(row, &format!("{base}_rating_normalized"))?.and_then(|v| v.parse().ok());
        Ok((rating, normalized))
    };

    let source = MangaBakaSourceDto {
        anilist: text(row, "source_anilist_id")?.map(|v| {
            let (rating, normalized) = rating_col(row, "source_anilist").unwrap_or((None, None));
            MangaBakaSourceEntryDto {
                id: Some(serde_json::Value::from(v.parse::<i64>().unwrap_or(0))),
                rating,
                rating_normalized: normalized,
            }
        }),
        anime_news_network: text(row, "source_anime_news_network_id")?.map(|v| {
            let (rating, normalized) = rating_col(row, "source_anime_news_network").unwrap_or((None, None));
            MangaBakaSourceEntryDto {
                id: Some(serde_json::Value::from(v.parse::<i64>().unwrap_or(0))),
                rating,
                rating_normalized: normalized,
            }
        }),
        anime_planet: text(row, "source_anime_planet_id")?.map(|v| {
            let (rating, normalized) = rating_col(row, "source_anime_planet").unwrap_or((None, None));
            MangaBakaSourceEntryDto {
                id: Some(serde_json::Value::String(v)),
                rating,
                rating_normalized: normalized,
            }
        }),
        kitsu: text(row, "source_kitsu_id")?.map(|v| {
            let (rating, normalized) = rating_col(row, "source_kitsu").unwrap_or((None, None));
            MangaBakaSourceEntryDto {
                id: Some(serde_json::Value::from(v.parse::<i64>().unwrap_or(0))),
                rating,
                rating_normalized: normalized,
            }
        }),
        manga_updates: text(row, "source_manga_updates_id")?.map(|v| {
            let (rating, normalized) = rating_col(row, "source_manga_updates").unwrap_or((None, None));
            MangaBakaSourceEntryDto {
                id: Some(serde_json::Value::String(v)),
                rating,
                rating_normalized: normalized,
            }
        }),
        my_anime_list: text(row, "source_my_anime_list_id")?.map(|v| {
            let (rating, normalized) = rating_col(row, "source_my_anime_list").unwrap_or((None, None));
            MangaBakaSourceEntryDto {
                id: Some(serde_json::Value::from(v.parse::<i64>().unwrap_or(0))),
                rating,
                rating_normalized: normalized,
            }
        }),
        shikimori: text(row, "source_shikimori_id")?.map(|v| {
            let (rating, normalized) = rating_col(row, "source_shikimori").unwrap_or((None, None));
            MangaBakaSourceEntryDto {
                id: Some(serde_json::Value::from(v.parse::<i64>().unwrap_or(0))),
                rating,
                rating_normalized: normalized,
            }
        }),
    };

    let published = match text(row, "published_start_date")? {
        Some(date) => Some(MangaBakaPublishedDateDto {
            start_date: Some(date),
            start_date_is_estimated: text(row, "published_start_date_is_estimated")?
                .and_then(|v| v.parse().ok()),
            end_date: text(row, "published_end_date")?,
            end_date_is_estimated: text(row, "published_end_date_is_estimated")?
                .and_then(|v| v.parse().ok()),
        }),
        None => None,
    };

    let anime = match (text(row, "anime_start")?, text(row, "anime_end")?) {
        (Some(_), _) | (_, Some(_)) => Some(MangaBakaAnimeInfoDto {
            start: text(row, "anime_start")?,
            end: text(row, "anime_end")?,
        }),
        _ => None,
    };

    Ok(MangaBakaSeriesDto {
        id: row
            .get("id")
            .map_err(|e| ProviderError::message(format!("MangaBaka db id: {e}")))?,
        has_anime: text(row, "has_anime")?.map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false),
        anime,
        content_rating: Some(content_rating),
        is_licensed: text(row, "is_licensed")?.map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false),
        last_updated_at: text(row, "last_updated_at")?,
        merged_with: text(row, "merged_with")?.and_then(|v| v.parse().ok()),
        original_language: text(row, "original_language")?,
        state: Some(state),
        total_chapters: text(row, "total_chapters")?,
        relationships_v2: json_vec(row, "relationships_v2")?
            .map(|v| {
                serde_json::from_value(v)
                    .map_err(|e| ProviderError::message(format!("MangaBaka db relationships: {e}")))
            })
            .transpose()?,
        artists: json_vec(row, "artists")?
            .map(|v| {
                serde_json::from_value(v)
                    .map_err(|e| ProviderError::message(format!("MangaBaka db artists: {e}")))
            })
            .transpose()?,
        authors: json_vec(row, "authors")?
            .map(|v| {
                serde_json::from_value(v)
                    .map_err(|e| ProviderError::message(format!("MangaBaka db authors: {e}")))
            })
            .transpose()?,
        canonical_url: text(row, "canonical_url")?.unwrap_or_default(),
        cover,
        description: text(row, "description")?,
        final_volume: text(row, "final_volume")?,
        publishers: json_vec(row, "publishers")?
            .map(|v| {
                serde_json::from_value(v)
                    .map_err(|e| ProviderError::message(format!("MangaBaka db publishers: {e}")))
            })
            .transpose()?,
        rating: row
            .get("rating")
            .map_err(|e| ProviderError::message(format!("MangaBaka db rating: {e}")))?,
        status,
        r#type: type_,
        links_v2: json_vec(row, "links_v2")?
            .map(|v| {
                serde_json::from_value(v)
                    .map_err(|e| ProviderError::message(format!("MangaBaka db links: {e}")))
            })
            .transpose()?,
        published,
        tags_v2: json_vec(row, "tags_v2")?
            .map(|v| {
                serde_json::from_value(v)
                    .map_err(|e| ProviderError::message(format!("MangaBaka db tags: {e}")))
            })
            .transpose()?,
        titles: json_vec(row, "titles")?
            .map(|v| {
                serde_json::from_value(v)
                    .map_err(|e| ProviderError::message(format!("MangaBaka db titles: {e}")))
            })
            .transpose()?,
        source,
    })
}

// ---------------------------------------------------------------------------
// DATABASE 下载器 —— 对应 `MangaBakaDbDownloader.kt` / `MangaBakaDbDownloadTimestamp.kt`
// ---------------------------------------------------------------------------

/// MangaBaka 本地数据库下载器：sha1 校验 → 下载 tar.gz → 解压首个条目 → 建 FTS5 索引。
///
/// 元数据（timestamp / checksum）存于 `mangabaka/` 目录下，对应 Kotlin
/// `MangaBakaDbMetadata`（timestamp、checksum.sha1 文件）。
pub struct MangaBakaDbDownloader {
    pub work_dir: PathBuf,
    pub database_archive: PathBuf,
    pub database_file: PathBuf,
    pub http: reqwest::Client,
    pub download_in_progress: Arc<std::sync::atomic::AtomicBool>,
    pub progress: Arc<std::sync::Mutex<Option<tokio::sync::watch::Sender<Option<DownloadProgress>>>>>,
}

impl MangaBakaDbDownloader {
    pub fn new(work_dir: impl Into<PathBuf>, http: reqwest::Client) -> Self {
        let work_dir = work_dir.into();
        Self {
            database_file: work_dir.join("mangabaka.sqlite"),
            database_archive: work_dir.join("mangabaka.tar.gz"),
            work_dir,
            http,
            download_in_progress: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            progress: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    pub fn database_file(&self) -> PathBuf {
        self.database_file.clone()
    }

    /// 工作目录（timestamp / checksum 文件所在目录）。
    pub fn work_dir(&self) -> &PathBuf {
        &self.work_dir
    }

    /// 对应 Kotlin `MangaBakaDbMetadata.timestamp`（`/config` 返回用）。
    pub fn download_timestamp(&self) -> Option<String> {
        std::fs::read_to_string(self.work_dir.join("timestamp"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// 对应 Kotlin `MangaBakaDbMetadata.isValid()`：timestamp + checksum 文件均存在。
    fn metadata_valid(&self) -> bool {
        self.work_dir.join("timestamp").exists() && self.work_dir.join("checksum.sha1").exists()
    }

    /// 启动（或复用进行中的）下载，返回进度事件流。
    pub fn launch_download(&self) -> tokio::sync::watch::Receiver<Option<DownloadProgress>> {
        let (sender, receiver) = tokio::sync::watch::channel(None);
        if self
            .download_in_progress
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_ok()
        {
            {
                let mut guard = self.progress.lock().unwrap();
                *guard = Some(sender.clone());
            }
            let this = self.clone();
            tokio::spawn(async move {
                let result = this.do_download(&sender).await;
                this.download_in_progress
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                let _ = result;
            });
        } else {
            let guard = self.progress.lock().unwrap();
            if let Some(current) = guard.as_ref() {
                return current.subscribe();
            }
        }
        receiver
    }

    async fn do_download(
        &self,
        sender: &tokio::sync::watch::Sender<Option<DownloadProgress>>,
    ) -> Result<(), String> {
        let emit = |sender: &tokio::sync::watch::Sender<Option<DownloadProgress>>,
                    event: DownloadProgress| {
            let _ = sender.send(Some(event));
        };

        // 1. 校验和检查：db 有效且校验一致 → 直接 Finished 跳过下载
        emit(
            sender,
            DownloadProgress::ProgressEvent {
                total: 0,
                completed: 0,
                info: Some(MANGA_BAKA_CHECKSUM_URL.to_string()),
            },
        );
        let new_checksum = self
            .http
            .get(MANGA_BAKA_CHECKSUM_URL)
            .send()
            .await
            .map_err(|e| format!("HttpRequestException: {e}"))?
            .text()
            .await
            .map_err(|e| format!("HttpRequestException: {e}"))?
            .trim()
            .to_string();
        if self.database_file.exists() && self.metadata_valid() {
            let stored = std::fs::read_to_string(self.work_dir.join("checksum.sha1"))
                .unwrap_or_default()
                .trim()
                .to_string();
            if stored == new_checksum {
                emit(sender, DownloadProgress::FinishedEvent);
                return Ok(());
            }
        }
        // 元数据/旧库清理
        let _ = std::fs::remove_file(self.work_dir.join("timestamp"));
        let _ = std::fs::remove_file(self.work_dir.join("checksum.sha1"));
        let _ = std::fs::remove_file(&self.database_file);
        let _ = std::fs::remove_file(&self.database_archive);
        let _ = std::fs::create_dir_all(&self.work_dir);

        // 2. 下载压缩包
        emit(
            sender,
            DownloadProgress::ProgressEvent {
                total: 0,
                completed: 0,
                info: Some(MANGA_BAKA_DB_URL.to_string()),
            },
        );
        let response = self
            .http
            .get(MANGA_BAKA_DB_URL)
            .send()
            .await
            .map_err(|e| format!("HttpRequestException: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("ResponseException: {}", response.status()));
        }
        let total = response.content_length().unwrap_or(0) as i64;
        emit(
            sender,
            DownloadProgress::ProgressEvent {
                total,
                completed: 0,
                info: Some(MANGA_BAKA_DB_URL.to_string()),
            },
        );
        let mut stream = response.bytes_stream();
        let mut file = tokio::fs::File::create(&self.database_archive)
            .await
            .map_err(|e| format!("FileSystemException: {e}"))?;
        let mut completed: i64 = 0;
        use futures::StreamExt;
        use tokio::io::AsyncWriteExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("HttpRequestException: {e}"))?;
            completed += chunk.len() as i64;
            file.write_all(&chunk)
                .await
                .map_err(|e| format!("FileSystemException: {e}"))?;
            emit(
                sender,
                DownloadProgress::ProgressEvent {
                    total,
                    completed,
                    info: Some(MANGA_BAKA_DB_URL.to_string()),
                },
            );
        }
        file.flush()
            .await
            .map_err(|e| format!("FileSystemException: {e}"))?;
        drop(file);

        // 3. 解压（tar + gzip，取首个条目）——对应 Kotlin `extractDatabaseFile`
        emit(
            sender,
            DownloadProgress::ProgressEvent {
                total: 0,
                completed: 0,
                info: Some(format!("extracting {:?}", self.database_archive)),
            },
        );
        {
            let input = std::fs::File::open(&self.database_archive)
                .map_err(|e| format!("FileSystemException: {e}"))?;
            let gz = flate2::read::GzDecoder::new(input);
            let mut archive = tar::Archive::new(gz);
            let mut entries = archive
                .entries()
                .map_err(|e| format!("TarException: {e}"))?;
            let first = entries
                .next()
                .ok_or_else(|| "TarException: empty archive".to_string())?
                .map_err(|e| format!("TarException: {e}"))?;
            let output = std::fs::File::create(&self.database_file)
                .map_err(|e| format!("FileSystemException: {e}"))?;
            let mut output = std::io::BufWriter::new(output);
            use std::io::Read;
            use std::io::Write;
            std::io::copy(&mut first.take(u64::MAX), &mut output)
                .map_err(|e| format!("FileSystemException: {e}"))?;
            output
                .flush()
                .map_err(|e| format!("FileSystemException: {e}"))?;
        }

        // 4. 重建自建表（对应上游 `prepareTables`）：数据表 tags/series_tags 每次重建，
        //    komga_series 关联数据保留；顺带清理旧版 series_fts（GROUP_CONCAT 结构）。
        emit(
            sender,
            DownloadProgress::ProgressEvent {
                total: 0,
                completed: 0,
                info: Some("preparing tables".to_string()),
            },
        );
        {
            let conn = rusqlite::Connection::open(&self.database_file)
                .map_err(|e| format!("SQLiteException: {e}"))?;
            conn.execute_batch(
                "DROP TABLE IF EXISTS titles_fts;
                 DROP TABLE IF EXISTS series_fts;
                 DROP TABLE IF EXISTS tags;
                 DROP TABLE IF EXISTS series_tags;",
            )
            .map_err(|e| format!("SQLiteException: {e}"))?;
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS tags (
                    id INTEGER PRIMARY KEY,
                    parent_id INTEGER,
                    merged_with INTEGER,
                    name TEXT NOT NULL,
                    name_path TEXT NOT NULL DEFAULT '',
                    description TEXT,
                    is_spoiler INTEGER NOT NULL DEFAULT 0,
                    is_genre INTEGER NOT NULL DEFAULT 0,
                    content_rating TEXT NOT NULL,
                    series_count INTEGER NOT NULL DEFAULT 0,
                    level INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE TABLE IF NOT EXISTS series_tags (
                    series_id INTEGER NOT NULL,
                    tag_id INTEGER NOT NULL,
                    is_spoiler INTEGER NOT NULL DEFAULT 0,
                    is_explicit INTEGER NOT NULL DEFAULT 0,
                    implied_by_tag_ids TEXT NOT NULL DEFAULT '[]',
                    weight TEXT NOT NULL DEFAULT 'unweighted',
                    PRIMARY KEY (tag_id, series_id)
                 );
                 CREATE TABLE IF NOT EXISTS komga_series (
                    komga_id TEXT NOT NULL,
                    mangabaka_id INTEGER NOT NULL,
                    PRIMARY KEY (komga_id, mangabaka_id)
                 );",
            )
            .map_err(|e| format!("SQLiteException: {e}"))?;
            conn.execute_batch("VACUUM;")
                .map_err(|e| format!("SQLiteException: {e}"))?;
        }

        // 5. 导入标签目录（对应上游 `importTags`：GET /v1/tags 全量拉取）。
        emit(
            sender,
            DownloadProgress::ProgressEvent {
                total: 0,
                completed: 0,
                info: Some("importing tags".to_string()),
            },
        );
        {
            let response = self
                .http
                .get("https://api.mangabaka.org/v1/tags")
                .send()
                .await
                .map_err(|e| format!("HttpRequestException: {e}"))?;
            if !response.status().is_success() {
                return Err(format!("ResponseException: {}", response.status()));
            }
            let response: MangaBakaTagsResponse = response
                .json()
                .await
                .map_err(|e| format!("SerializationException: {e}"))?;
            let conn = rusqlite::Connection::open(&self.database_file)
                .map_err(|e| format!("SQLiteException: {e}"))?;
            for tag in response.data {
                conn.execute(
                    "INSERT OR REPLACE INTO tags
                     (id, parent_id, merged_with, name, name_path, description, is_spoiler, is_genre, content_rating, series_count, level)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    rusqlite::params![
                        tag.id,
                        tag.parent_id,
                        tag.merged_with,
                        tag.name,
                        tag.name_path,
                        tag.description,
                        tag.is_spoiler.map(|v| if v { 1 } else { 0 }),
                        if tag.is_genre { 1 } else { 0 },
                        tag.content_rating
                            .map(|v| v.as_db_str())
                            .unwrap_or("safe"),
                        tag.series_count,
                        tag.level,
                    ],
                )
                .map_err(|e| format!("SQLiteException: {e}"))?;
            }
        }

        // 6. 导入系列-标签关联（对应上游 `importData` 的 series_tags INSERT）。
        emit(
            sender,
            DownloadProgress::ProgressEvent {
                total: 0,
                completed: 0,
                info: Some("importing series tags".to_string()),
            },
        );
        {
            let conn = rusqlite::Connection::open(&self.database_file)
                .map_err(|e| format!("SQLiteException: {e}"))?;
            conn.execute_batch(
                "INSERT INTO series_tags
                 SELECT s.id,
                        json_each.value ->> '$.id',
                        json_each.value ->> '$.is_spoiler',
                        json_each.value ->> '$.is_explicit',
                        json_each.value ->> '$.implied_by_tag_ids',
                        UPPER(json_each.value ->> '$.weight')
                 FROM series s, json_each(s.tags_v2)
                 WHERE s.state = 'active';",
            )
            .map_err(|e| format!("SQLiteException: {e}"))?;
        }

        // 7. FTS5 索引（对应上游 `createSearchIndex`：titles_fts 每标题一行，标题级 rank）。
        emit(
            sender,
            DownloadProgress::ProgressEvent {
                total: 0,
                completed: 0,
                info: Some("creating search index".to_string()),
            },
        );
        {
            let conn = rusqlite::Connection::open(&self.database_file)
                .map_err(|e| format!("SQLiteException: {e}"))?;
            conn.execute_batch(
                "CREATE VIRTUAL TABLE titles_fts USING fts5
                 (id, title, type, tokenize = 'trigram');",
            )
            .map_err(|e| format!("SQLiteException: {e}"))?;
            conn.execute_batch(
                "INSERT INTO titles_fts
                 SELECT s.id, json_each.value ->> '$.title', s.type
                 FROM series s, json_each(s.titles)
                 WHERE s.state = 'active';",
            )
            .map_err(|e| format!("SQLiteException: {e}"))?;
        }

        // 5. 写元数据 + 清理压缩包
        let now = chrono::Utc::now();
        let ts = now.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
        std::fs::write(self.work_dir.join("timestamp"), ts)
            .map_err(|e| format!("FileSystemException: {e}"))?;
        std::fs::write(self.work_dir.join("checksum.sha1"), &new_checksum)
            .map_err(|e| format!("FileSystemException: {e}"))?;
        let _ = std::fs::remove_file(&self.database_archive);

        emit(sender, DownloadProgress::FinishedEvent);
        Ok(())
    }
}

impl Clone for MangaBakaDbDownloader {
    fn clone(&self) -> Self {
        Self {
            work_dir: self.work_dir.clone(),
            database_archive: self.database_archive.clone(),
            database_file: self.database_file.clone(),
            http: self.http.clone(),
            download_in_progress: self.download_in_progress.clone(),
            progress: self.progress.clone(),
        }
    }
}
// ---------------------------------------------------------------------------
// 管理 API 映射（core DTO -> komf-api-models DTO，对应 Kotlin `MangaBakaMapper.kt`）
// ---------------------------------------------------------------------------

fn api_content_rating(v: Option<MangaBakaContentRatingDto>) -> komf_api_models::mangabaka::MangaBakaContentRating {
    match v.unwrap_or_default() {
        MangaBakaContentRatingDto::Safe => komf_api_models::mangabaka::MangaBakaContentRating::Safe,
        MangaBakaContentRatingDto::Suggestive => {
            komf_api_models::mangabaka::MangaBakaContentRating::Suggestive
        }
        MangaBakaContentRatingDto::Erotica => komf_api_models::mangabaka::MangaBakaContentRating::Erotica,
        MangaBakaContentRatingDto::Pornographic => {
            komf_api_models::mangabaka::MangaBakaContentRating::Pornographic
        }
    }
}

fn api_status(v: MangaBakaStatusDto) -> komf_api_models::mangabaka::MangaBakaStatus {
    match v {
        MangaBakaStatusDto::Cancelled => komf_api_models::mangabaka::MangaBakaStatus::Cancelled,
        MangaBakaStatusDto::Completed => komf_api_models::mangabaka::MangaBakaStatus::Completed,
        MangaBakaStatusDto::Hiatus => komf_api_models::mangabaka::MangaBakaStatus::Hiatus,
        MangaBakaStatusDto::Releasing => komf_api_models::mangabaka::MangaBakaStatus::Releasing,
        MangaBakaStatusDto::Upcoming => komf_api_models::mangabaka::MangaBakaStatus::Upcoming,
        MangaBakaStatusDto::Unknown => komf_api_models::mangabaka::MangaBakaStatus::Unknown,
    }
}

fn api_type(v: MangaBakaTypeDto) -> komf_api_models::mangabaka::MangaBakaType {
    match v {
        MangaBakaTypeDto::Manga => komf_api_models::mangabaka::MangaBakaType::Manga,
        MangaBakaTypeDto::Novel => komf_api_models::mangabaka::MangaBakaType::Novel,
        MangaBakaTypeDto::Manhwa => komf_api_models::mangabaka::MangaBakaType::Manhwa,
        MangaBakaTypeDto::Manhua => komf_api_models::mangabaka::MangaBakaType::Manhua,
        MangaBakaTypeDto::Oel => komf_api_models::mangabaka::MangaBakaType::Oel,
        MangaBakaTypeDto::Other => komf_api_models::mangabaka::MangaBakaType::Other,
    }
}

fn api_state(v: Option<MangaBakaSeriesStateDto>) -> komf_api_models::mangabaka::MangaBakaSeriesState {
    match v.unwrap_or_default() {
        MangaBakaSeriesStateDto::Active => komf_api_models::mangabaka::MangaBakaSeriesState::Active,
        MangaBakaSeriesStateDto::Merged => komf_api_models::mangabaka::MangaBakaSeriesState::Merged,
        MangaBakaSeriesStateDto::Deleted => komf_api_models::mangabaka::MangaBakaSeriesState::Deleted,
    }
}

fn api_link_type(v: Option<MangaBakaLinkTypeDto>) -> komf_api_models::mangabaka::MangaBakaLinkType {
    match v.unwrap_or(MangaBakaLinkTypeDto::Other) {
        MangaBakaLinkTypeDto::Publisher => komf_api_models::mangabaka::MangaBakaLinkType::Publisher,
        MangaBakaLinkTypeDto::Retailer => komf_api_models::mangabaka::MangaBakaLinkType::Retailer,
        MangaBakaLinkTypeDto::Webplatform => komf_api_models::mangabaka::MangaBakaLinkType::Webplatform,
        MangaBakaLinkTypeDto::Info => komf_api_models::mangabaka::MangaBakaLinkType::Info,
        MangaBakaLinkTypeDto::Social => komf_api_models::mangabaka::MangaBakaLinkType::Social,
        MangaBakaLinkTypeDto::News => komf_api_models::mangabaka::MangaBakaLinkType::News,
        MangaBakaLinkTypeDto::Piracy => komf_api_models::mangabaka::MangaBakaLinkType::Piracy,
        MangaBakaLinkTypeDto::Other => komf_api_models::mangabaka::MangaBakaLinkType::Other,
    }
}

fn api_relation_type(v: MangaBakaRelationTypeDto) -> komf_api_models::mangabaka::MangaBakaRelationType {
    use komf_api_models::mangabaka::MangaBakaRelationType as A;
    match v {
        MangaBakaRelationTypeDto::Adaptation => A::Adaptation,
        MangaBakaRelationTypeDto::Alternative => A::Alternative,
        MangaBakaRelationTypeDto::Cameo => A::Cameo,
        MangaBakaRelationTypeDto::CharacterFocus => A::CharacterFocus,
        MangaBakaRelationTypeDto::Compilation => A::Compilation,
        MangaBakaRelationTypeDto::Contains => A::Contains,
        MangaBakaRelationTypeDto::Crossover => A::Crossover,
        MangaBakaRelationTypeDto::Expansion => A::Expansion,
        MangaBakaRelationTypeDto::Main => A::Main,
        MangaBakaRelationTypeDto::Other => A::Other,
        MangaBakaRelationTypeDto::Parent => A::Parent,
        MangaBakaRelationTypeDto::Parody => A::Parody,
        MangaBakaRelationTypeDto::Prequel => A::Prequel,
        MangaBakaRelationTypeDto::Reboot => A::Reboot,
        MangaBakaRelationTypeDto::Remake => A::Remake,
        MangaBakaRelationTypeDto::Sequel => A::Sequel,
        MangaBakaRelationTypeDto::Series => A::Series,
        MangaBakaRelationTypeDto::SideStory => A::SideStory,
        MangaBakaRelationTypeDto::Source => A::Source,
        MangaBakaRelationTypeDto::SpinOff => A::SpinOff,
        MangaBakaRelationTypeDto::Summary => A::Summary,
        MangaBakaRelationTypeDto::Uncollected => A::Uncollected,
    }
}

fn api_chronology(v: MangaBakaRelationshipChronologyDto) -> komf_api_models::mangabaka::MangaBakaRelationshipChronology {
    match v {
        MangaBakaRelationshipChronologyDto::Narrative => {
            komf_api_models::mangabaka::MangaBakaRelationshipChronology::Narrative
        }
        MangaBakaRelationshipChronologyDto::Release => {
            komf_api_models::mangabaka::MangaBakaRelationshipChronology::Release
        }
        MangaBakaRelationshipChronologyDto::Unknown => {
            komf_api_models::mangabaka::MangaBakaRelationshipChronology::Unknown
        }
    }
}

fn api_weight(v: Option<MangaBakaTagWeightDto>) -> komf_api_models::mangabaka::MangaBakaTagWeight {
    match v.unwrap_or(MangaBakaTagWeightDto::Unweighted) {
        MangaBakaTagWeightDto::Core => komf_api_models::mangabaka::MangaBakaTagWeight::Core,
        MangaBakaTagWeightDto::Defining => komf_api_models::mangabaka::MangaBakaTagWeight::Defining,
        MangaBakaTagWeightDto::Recurrent => komf_api_models::mangabaka::MangaBakaTagWeight::Recurrent,
        MangaBakaTagWeightDto::Incidental => komf_api_models::mangabaka::MangaBakaTagWeight::Incidental,
        MangaBakaTagWeightDto::Unweighted => komf_api_models::mangabaka::MangaBakaTagWeight::Unweighted,
    }
}

fn api_title_trait(v: &str) -> komf_api_models::mangabaka::MangaBakaTitleTrait {
    match v.to_lowercase().as_str() {
        "official" => komf_api_models::mangabaka::MangaBakaTitleTrait::Official,
        "native" => komf_api_models::mangabaka::MangaBakaTitleTrait::Native,
        _ => komf_api_models::mangabaka::MangaBakaTitleTrait::Alternative,
    }
}

fn api_source_id(v: &Option<serde_json::Value>) -> Option<i64> {
    v.as_ref().and_then(|v| match v {
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    })
}

fn api_source_id_string(v: &Option<serde_json::Value>) -> Option<String> {
    v.as_ref().and_then(|v| match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

macro_rules! map_source_field {
    ($source:expr, $ty:ident, $conv:expr) => {{
        let entry = $source.as_ref();
        $ty {
            id: $conv(&entry.and_then(|e| e.id.clone())),
            rating: entry.and_then(|e| e.rating),
            rating_normalized: entry.and_then(|e| e.rating_normalized),
        }
    }};
}

/// `MangaBakaSeries.toDto()`：core DTO -> 管理 API DTO（对应 Kotlin MangaBakaMapper）。
pub fn to_api_series(series: &MangaBakaSeriesDto) -> komf_api_models::mangabaka::KomfMangaBakaSeries {
    use komf_api_models::mangabaka::*;
    KomfMangaBakaSeries {
        id: MangaBakaSeriesId(series.id),
        has_anime: series.has_anime,
        anime: series.anime.as_ref().map(|a| MangaBakaAnimeInfo {
            start: a.start.clone(),
            end: a.end.clone(),
        }),
        artists: series.artists.clone(),
        authors: series.authors.clone(),
        canonical_url: series.canonical_url.clone(),
        content_rating: api_content_rating(series.content_rating),
        cover: MangaBakaCover {
            raw: series.cover.raw.as_ref().map(|raw| MangaBakaCoverRaw {
                url: raw.url.clone(),
                size: raw.size,
                height: raw.height,
                width: raw.width,
                blurhash: raw.blurhash.clone(),
                thumbhash: raw.thumbhash.clone(),
                format: raw.format.clone(),
            }),
            x150: series.cover.x150.as_ref().map(map_dpi),
            x250: series.cover.x250.as_ref().map(map_dpi),
            x350: series.cover.x350.as_ref().map(map_dpi),
        },
        description: series.description.clone(),
        final_volume: series.final_volume.clone(),
        is_licensed: series.is_licensed,
        last_updated_at: series.last_updated_at.clone(),
        merged_with: series.merged_with,
        original_language: series.original_language.clone(),
        publishers: series.publishers.as_ref().map(|pubs| {
            pubs.iter()
                .map(|p| MangaBakaPublisher {
                    name: p.name.clone(),
                    note: p.note.clone(),
                    r#type: p.type_.clone(),
                })
                .collect()
        }),
        rating: series.rating,
        state: api_state(series.state),
        status: api_status(series.status),
        total_chapters: series.total_chapters.clone(),
        r#type: api_type(series.r#type),
        links: series.links_v2.as_ref().map(|links| {
            links
                .iter()
                .map(|l| MangaBakaLink {
                    id: MangaBakaLinkId(l.id.clone().unwrap_or_default()),
                    language: l.language.clone().unwrap_or_default(),
                    name: l.name.clone().unwrap_or_default(),
                    name_display: l.name_display.clone(),
                    r#type: api_link_type(l.type_),
                    url: l.url.clone(),
                })
                .collect()
        }),
        published: series.published.as_ref().map(|p| MangaBakaPublishedDate {
            end_date: p.end_date.clone(),
            end_date_is_estimated: p.end_date_is_estimated,
            start_date: p.start_date.clone(),
            start_date_is_estimated: p.start_date_is_estimated,
        }),
        relationships: series.relationships_v2.as_ref().map(|rels| {
            rels.iter()
                .map(|r| MangaBakaRelationship {
                    id: MangaBakaRelationshipId(r.id.clone()),
                    chronology: api_chronology(r.chronology),
                    is_manual: r.is_manual,
                    note: r.note.clone(),
                    relation_type: api_relation_type(r.relation_type),
                    to_series_id: MangaBakaSeriesId(r.to_series_id),
                })
                .collect()
        }),
        tags: series.tags_v2.as_ref().map(|tags| {
            tags.iter()
                .map(|t| MangaBakaSeriesTag {
                    id: MangaBakaTagId(t.id),
                    content_rating: api_content_rating(t.content_rating),
                    description: t.description.clone(),
                    is_spoiler: t.is_spoiler,
                    level: t.level,
                    name: t.name.clone(),
                    name_path: t.name_path.clone(),
                    parent_id: t.parent_id.map(MangaBakaTagId),
                    series_count: t.series_count,
                    implied_by_tag_ids: t.implied_by_tag_ids.iter().map(|id| MangaBakaTagId(*id)).collect(),
                    is_explicit: t.is_explicit,
                    is_genre: t.is_genre,
                    merged_with: t.merged_with,
                    weight: api_weight(t.weight),
                })
                .collect()
        }),
        titles: series.titles.as_ref().map(|titles| {
            titles
                .iter()
                .map(|t| MangaBakaTitle {
                    language: t.language.clone(),
                    title: t.title.clone(),
                    traits: t.traits.iter().map(|v| api_title_trait(v)).collect(),
                    is_primary: t.is_primary,
                    note: t.note.clone(),
                })
                .collect()
        }),
        source: MangaBakaSource {
            anilist: map_source_field!(series.source.anilist, MangaBakaAniListSource, api_source_id),
            anime_news_network: map_source_field!(
                series.source.anime_news_network,
                MangaBakaAnimeNewsNetworkSource,
                api_source_id
            ),
            anime_planet: map_source_field!(
                series.source.anime_planet,
                MangaBakaAnimePlanetSource,
                api_source_id_string
            ),
            kitsu: map_source_field!(series.source.kitsu, MangaBakaKitsuSource, api_source_id),
            manga_updates: map_source_field!(
                series.source.manga_updates,
                MangaBakaMangaUpdatesSource,
                api_source_id_string
            ),
            my_anime_list: map_source_field!(
                series.source.my_anime_list,
                MangaBakaMyAnimeListSource,
                api_source_id
            ),
            shikimori: map_source_field!(series.source.shikimori, MangaBakaShikimoriSource, api_source_id),
        },
    }
}

fn map_dpi(dpi: &MangaBakaCoverDpiDto) -> komf_api_models::mangabaka::MangaBakaCoverDpi {
    komf_api_models::mangabaka::MangaBakaCoverDpi {
        x1: dpi.x1.clone(),
        x2: dpi.x2.clone(),
        x3: dpi.x3.clone(),
    }
}

/// `MangaBakaTag.toDto()`：标签目录（GET /series/tags 响应）。
pub fn to_api_tag(tag: &MangaBakaTagDto) -> komf_api_models::mangabaka::KomfMangaBakaTag {
    komf_api_models::mangabaka::KomfMangaBakaTag {
        id: komf_api_models::mangabaka::MangaBakaTagId(tag.id),
        content_rating: api_content_rating(Some(tag.content_rating)),
        description: tag.description.clone(),
        is_spoiler: tag.is_spoiler,
        level: tag.level,
        name: tag.name.clone(),
        name_path: tag.name_path.clone(),
        parent_id: tag.parent_id.map(komf_api_models::mangabaka::MangaBakaTagId),
        series_count: tag.series_count,
        is_genre: tag.is_genre,
        merged_with: tag.merged_with,
    }
}

/// `MangaBakaLinkedSeries.toDto()`：Komga 系列与其 MangaBaka 系列的关联。
pub fn to_api_linked(linked: &MangaBakaLinkedSeriesDto) -> komf_api_models::mangabaka::KomfMangaBakaLinkedSeries {
    komf_api_models::mangabaka::KomfMangaBakaLinkedSeries {
        komga_id: komf_api_models::common::KomfServerSeriesId(linked.komga_id.clone()),
        manga_baka: to_api_series(&linked.series),
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping() {
        assert_eq!(
            map_status(&MangaBakaStatusDto::Releasing),
            SeriesStatus::Ongoing
        );
        assert_eq!(
            map_status(&MangaBakaStatusDto::Completed),
            SeriesStatus::Completed
        );
        assert_eq!(
            map_status(&MangaBakaStatusDto::Cancelled),
            SeriesStatus::Abandoned
        );
        assert_eq!(
            map_status(&MangaBakaStatusDto::Hiatus),
            SeriesStatus::Hiatus
        );
        assert_eq!(
            map_status(&MangaBakaStatusDto::Unknown),
            SeriesStatus::Ongoing
        );
    }

    #[test]
    fn html_to_text_strips_tags_and_entities() {
        assert_eq!(
            html_to_text("<p>Hello <b>world</b> &amp; more</p>"),
            "Hello world & more"
        );
        assert_eq!(html_to_text("plain text"), "plain text");
    }

    #[test]
    fn date_parsing() {
        assert_eq!(
            parse_date_parts("2020-05-12"),
            Some((2020, Some(5), Some(12)))
        );
        assert_eq!(parse_date_parts("garbage"), None);
    }

    #[test]
    fn official_link_url_parses_and_encodes() {
        assert_eq!(
            official_link_url("https://example.com/a b/日本語?q=1 2#frag ment"),
            Some(
                "https://example.com/a%20b/%E6%97%A5%E6%9C%AC%E8%AA%9E?q=1%202#frag%20ment"
                    .to_string()
            )
        );
        assert_eq!(
            official_link_url("https://example.com/path%20already?a=%41"),
            Some("https://example.com/path%20already?a=%41".to_string())
        );
        assert_eq!(official_link_url("not a url"), None);
        assert_eq!(
            official_link_url("https://example.com"),
            Some("https://example.com".to_string())
        );
        assert_eq!(
            official_link_url("https://example.com/"),
            Some("https://example.com/".to_string())
        );
    }

    #[test]
    fn primary_title_selection() {
        let series = MangaBakaSeriesDto {
            id: 1,
            artists: None,
            authors: None,
            canonical_url: "https://mangabaka.org/1".to_string(),
            cover: MangaBakaCoverDto::default(),
            description: None,
            final_volume: None,
            publishers: None,
            rating: None,
            status: MangaBakaStatusDto::Releasing,
            r#type: MangaBakaTypeDto::Manga,
            links_v2: None,
            published: None,
            tags_v2: None,
            has_anime: false,
            anime: None,
            content_rating: None,
            is_licensed: false,
            last_updated_at: None,
            merged_with: None,
            original_language: None,
            state: None,
            total_chapters: None,
            relationships_v2: None,
            titles: Some(vec![
                MangaBakaTitleDto {
                    language: "en".to_string(),
                    title: "English Title".to_string(),
                    traits: vec!["romanized".to_string()],
                    is_primary: Some(true),
                    note: None,
                },
                MangaBakaTitleDto {
                    language: "ja".to_string(),
                    title: "日本語".to_string(),
                    traits: vec!["native".to_string()],
                    is_primary: None,
                    note: None,
                },
            ]),
            source: MangaBakaSourceDto::default(),
        };
        assert_eq!(primary_title(&series, None), "日本語");
        assert_eq!(primary_title(&series, Some("en")), "English Title");
        assert_eq!(primary_title(&series, Some("zh")), "日本語");
    }

    #[test]
    fn deserializes_series_dto() {
        let json = r#"{
            "id": 123,
            "canonical_url": "https://mangabaka.org/123",
            "cover": {"x350": {"x1": "https://example.com/c.jpg"}},
            "status": "releasing",
            "type": "manga",
            "description": "<p>Summary</p>",
            "titles": [{"language": "en", "title": "Test", "traits": ["romanized"], "is_primary": true}],
            "source": {"anilist": {"id": 42}, "manga_updates": {"id": "abc"}},
            "tags_v2": [{"name": "Action", "is_genre": true}, {"name": "Nudity", "is_genre": false}],
            "published": {"start_date": "2018-01-02"}
        }"#;
        let series: MangaBakaSeriesDto = serde_json::from_str(json).unwrap();
        assert_eq!(series.id, 123);
        assert_eq!(series.status as i32, MangaBakaStatusDto::Releasing as i32);
        assert_eq!(
            series.cover.x350.as_ref().unwrap().x1.as_deref(),
            Some("https://example.com/c.jpg")
        );
        assert_eq!(
            series
                .source
                .anilist
                .as_ref()
                .unwrap()
                .id_string()
                .as_deref(),
            Some("42")
        );
        assert_eq!(
            series
                .source
                .manga_updates
                .as_ref()
                .unwrap()
                .id_string()
                .as_deref(),
            Some("abc")
        );
        assert_eq!(primary_title(&series, None), "Test");
    }

    /// 真实 SQLite 回环：komga_series 关联（link/unlink/find/find_all_linked）
    /// 与 tags 目录（get_all_tags + series_tags 全字段补全）。
    #[test]
    fn repository_linking_and_tags_roundtrip() {
        let dir = std::env::temp_dir().join(format!("komf-mangabaka-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("mangabaka.sqlite");

        let repo = MangaBakaDbRepository::new(db_path.clone());
        // Repository::open() 触发 ensure komga_series / tags / series_tags。
        assert!(repo.get_all_tags().unwrap().is_empty());
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE series (
                id INTEGER PRIMARY KEY,
                canonical_url TEXT,
                cover_x350_x1 TEXT,
                description TEXT,
                final_volume TEXT,
                publishers TEXT,
                rating REAL,
                status TEXT,
                type TEXT,
                links_v2 TEXT,
                published_start_date TEXT,
                published_start_date_is_estimated INTEGER,
                published_end_date TEXT,
                published_end_date_is_estimated INTEGER,
                tags_v2 TEXT,
                titles TEXT,
                artists TEXT,
                authors TEXT,
                has_anime INTEGER,
                anime_start TEXT,
                anime_end TEXT,
                content_rating TEXT,
                is_licensed INTEGER,
                last_updated_at TEXT,
                merged_with INTEGER,
                original_language TEXT,
                state TEXT,
                total_chapters TEXT,
                relationships_v2 TEXT,
                cover_raw_url TEXT,
                cover_raw_size INTEGER,
                cover_raw_height INTEGER,
                cover_raw_width INTEGER,
                cover_raw_blurhash TEXT,
                cover_raw_thumbhash TEXT,
                cover_raw_format TEXT,
                source_anilist_id TEXT,
                source_anilist_rating REAL,
                source_anilist_rating_normalized INTEGER,
                source_anime_news_network_id TEXT,
                source_anime_news_network_rating REAL,
                source_anime_news_network_rating_normalized INTEGER,
                source_anime_planet_id TEXT,
                source_anime_planet_rating REAL,
                source_anime_planet_rating_normalized INTEGER,
                source_kitsu_id TEXT,
                source_kitsu_rating REAL,
                source_kitsu_rating_normalized INTEGER,
                source_manga_updates_id TEXT,
                source_manga_updates_rating REAL,
                source_manga_updates_rating_normalized INTEGER,
                source_my_anime_list_id TEXT,
                source_my_anime_list_rating REAL,
                source_my_anime_list_rating_normalized INTEGER,
                source_shikimori_id TEXT,
                source_shikimori_rating REAL,
                source_shikimori_rating_normalized INTEGER
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO series (id, canonical_url, status, type, titles)
             VALUES (1, 'https://mangabaka.org/1', 'releasing', 'manga', '[]')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO series (id, canonical_url, status, type, titles)
             VALUES (2, 'https://mangabaka.org/2', 'completed', 'manga', '[]')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tags (id, name, name_path, content_rating, is_genre, series_count, level)
             VALUES (10, 'Action', '/Action', 'safe', 1, 5, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO series_tags (series_id, tag_id, weight, is_explicit)
             VALUES (1, 10, 'CORE', 0)",
            [],
        )
        .unwrap();

        // link + find（含 series_tags join 补全 tags_v2）
        repo.link("komga-1", 1).unwrap();
        let linked = repo.find("komga-1").unwrap().expect("linked series");
        assert_eq!(linked.komga_id, "komga-1");
        assert_eq!(linked.series.id, 1);
        let tags = linked.series.tags_v2.as_ref().expect("series tags");
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].name, "Action");
        assert!(tags[0].is_genre);
        assert_eq!(tags[0].weight, Some(MangaBakaTagWeightDto::Core));

        // find_all_linked 保持输入顺序、缺 id 跳过
        let all = repo
            .find_all_linked(&["komga-1".to_string(), "missing".to_string()])
            .unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].komga_id, "komga-1");

        // 重复 link 先删旧关联
        repo.link("komga-1", 2).unwrap();
        let linked = repo.find("komga-1").unwrap().expect("re-linked series");
        assert_eq!(linked.series.id, 2);

        // unlink
        repo.unlink("komga-1").unwrap();
        assert!(repo.find("komga-1").unwrap().is_none());

        // link 不存在的系列 -> 错误
        assert!(repo.link("komga-x", 999).is_err());

        // tags 目录
        let tags = repo.get_all_tags().unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].name, "Action");
        assert_eq!(tags[0].content_rating, MangaBakaContentRatingDto::Safe);
        assert!(tags[0].is_genre);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
