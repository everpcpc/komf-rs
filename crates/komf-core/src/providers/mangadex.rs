//! MangaDex provider —— 对应 `providers/mangadex` 包。
//!
//! 使用 MangaDex REST API v2（`https://api.mangadex.org`）。
use crate::config::MangaDexConfig;
use crate::model::{
    Author, AuthorRole, Image, MatchQuery, ProviderBookId, ProviderBookMetadata, ProviderSeriesId,
    ProviderSeriesMetadata, ReadingDirection, SeriesBook, SeriesMetadata, SeriesSearchResult,
    SeriesStatus, SeriesTitle, TitleType, WebLink,
};
use crate::providers::{CoreProviders, MetadataProvider, ProviderError};
use crate::util::NameSimilarityMatcher;
use serde::Deserialize;

const BASE_URL: &str = "https://api.mangadex.org";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MangaDexResponse<T> {
    pub result: String,
    pub data: T,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
    #[serde(default)]
    pub total: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MangaDexManga {
    pub id: String,
    pub r#type: String,
    pub attributes: MangaDexMangaAttributes,
    pub relationships: Vec<MangaDexRelationship>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MangaDexMangaAttributes {
    pub title: std::collections::HashMap<String, String>,
    pub alt_titles: Vec<std::collections::HashMap<String, String>>,
    pub description: std::collections::HashMap<String, String>,
    pub original_language: String,
    pub status: Option<String>,
    #[serde(default)]
    pub publication_demographic: Option<String>,
    pub year: Option<i32>,
    pub tags: Vec<MangaDexTag>,
    pub state: Option<String>,
    pub links: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MangaDexTag {
    pub id: String,
    pub attributes: MangaDexTagAttributes,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MangaDexTagAttributes {
    pub name: std::collections::HashMap<String, String>,
    pub group: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MangaDexRelationship {
    pub id: String,
    pub r#type: String,
    pub attributes: Option<MangaDexRelationshipAttributes>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MangaDexRelationshipAttributes {
    pub name: Option<String>,
    pub role: Option<String>,
    pub volume: Option<String>,
    pub chapter: Option<String>,
    pub title: Option<String>,
    pub locale: Option<String>,
    pub description: Option<String>,
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MangaDexChapter {
    pub id: String,
    pub attributes: MangaDexChapterAttributes,
    pub relationships: Vec<MangaDexRelationship>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MangaDexChapterAttributes {
    pub volume: Option<String>,
    pub chapter: Option<String>,
    pub title: Option<String>,
    pub translated_language: Option<String>,
    pub pages: Option<i32>,
}

pub struct MangaDexClient {
    http: reqwest::Client,
}

impl MangaDexClient {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }

    pub async fn search_series(
        &self,
        name: &str,
        limit: u32,
    ) -> Result<Vec<MangaDexManga>, ProviderError> {
        // 对齐 Kotlin MangaDexClient.searchSeries：
        // order[relevance]=desc、contentRating[]=safe/suggestive/erotica/pornographic（不过滤成人内容）
        let response = self
            .http
            .get(format!("{BASE_URL}/manga"))
            .query(&[
                ("title", name),
                ("limit", &limit.to_string()),
                ("includes[]", "cover_art"),
                ("includes[]", "author"),
                ("includes[]", "artist"),
                ("order[relevance]", "desc"),
                ("contentRating[]", "safe"),
                ("contentRating[]", "suggestive"),
                ("contentRating[]", "erotica"),
                ("contentRating[]", "pornographic"),
            ])
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::Status(CoreProviders::Mangadex, status, body));
        }
        let body: MangaDexResponse<Vec<MangaDexManga>> = response.json().await?;
        Ok(body.data)
    }

    pub async fn get_series(&self, id: &str) -> Result<MangaDexManga, ProviderError> {
        let response = self
            .http
            .get(format!("{BASE_URL}/manga/{id}"))
            .query(&[
                ("includes[]", "cover_art"),
                ("includes[]", "author"),
                ("includes[]", "artist"),
            ])
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::Status(CoreProviders::Mangadex, status, body));
        }
        let body: MangaDexResponse<MangaDexManga> = response.json().await?;
        Ok(body.data)
    }

    pub async fn get_chapters(
        &self,
        manga_id: &str,
        translated_language: &str,
    ) -> Result<Vec<MangaDexChapter>, ProviderError> {
        let response = self
            .http
            .get(format!("{BASE_URL}/manga/{manga_id}/feed"))
            .query(&[
                ("translatedLanguage[]", translated_language),
                ("limit", "500"),
                ("order[chapter]", "asc"),
            ])
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::Status(CoreProviders::Mangadex, status, body));
        }
        let body: MangaDexResponse<Vec<MangaDexChapter>> = response.json().await?;
        Ok(body.data)
    }

    pub async fn get_cover_url(&self, manga: &MangaDexManga) -> Option<String> {
        // 封面原始文件 URL（fileName 已含扩展名）。不带 .512.jpg 尺寸后缀：
        // 部分网络环境下 uploads CDN 对缩略图端点返回 MangaDex 占位横幅而非封面。
        let cover = manga
            .relationships
            .iter()
            .find(|r| r.r#type == "cover_art")?;
        let file_name = cover.attributes.as_ref()?.file_name.clone()?;
        Some(format!(
            "https://uploads.mangadex.org/covers/{}/{file_name}",
            manga.id
        ))
    }

    /// 对应 Kotlin getSeriesCovers：分页拉取某 manga 的封面列表。
    pub async fn get_series_covers(
        &self,
        manga_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<MangaDexResponse<Vec<MangaDexRelationship>>, ProviderError> {
        let response = self
            .http
            .get(format!("{BASE_URL}/cover"))
            .query(&[
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
                ("manga[]", &manga_id.to_string()),
            ])
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::Status(CoreProviders::Mangadex, status, body));
        }
        Ok(response.json().await?)
    }

    /// 对应 Kotlin getAllCovers：循环分页直到取完（最多 100 次请求）。
    pub async fn get_all_covers(
        &self,
        manga_id: &str,
    ) -> Result<Vec<MangaDexRelationship>, ProviderError> {
        let mut covers = Vec::new();
        let mut offset = 0u32;
        let mut request_count = 0;
        while request_count < 100 {
            let page = self.get_series_covers(manga_id, 100, offset).await?;
            let limit = page.limit.unwrap_or(100);
            let total = page.total.unwrap_or(0);
            let page_offset = page.offset.unwrap_or(offset);
            covers.extend(page.data);
            if page_offset + limit > total {
                break;
            }
            offset += limit;
            request_count += 1;
        }
        Ok(covers)
    }

    pub async fn get_thumbnail(&self, url: &str) -> Result<Option<Image>, ProviderError> {
        let response = self.http.get(url).send().await?;
        if !response.status().is_success() {
            return Ok(None);
        }
        let bytes = response.bytes().await?;
        Ok(Some(Image::new(bytes.to_vec(), None)))
    }
}

pub struct MangaDexMetadataMapper {
    series_metadata_config: crate::config::SeriesMetadataConfig,
    #[allow(dead_code)]
    book_metadata_config: crate::config::BookMetadataConfig,
    author_roles: Vec<AuthorRole>,
    artist_roles: Vec<AuthorRole>,
    cover_languages: Vec<String>,
    links_filter: Vec<crate::config::MangaDexLink>,
    /// 全局 postProcessing.seriesTitleLanguage 联动：搜索/写入时按该语言选主标题（仅显示，不参与匹配）
    series_title_language: Option<String>,
}

impl MangaDexMetadataMapper {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        series_metadata_config: crate::config::SeriesMetadataConfig,
        book_metadata_config: crate::config::BookMetadataConfig,
        author_roles: Vec<AuthorRole>,
        artist_roles: Vec<AuthorRole>,
        cover_languages: Vec<String>,
        links_filter: Vec<crate::config::MangaDexLink>,
        series_title_language: Option<String>,
    ) -> Self {
        Self {
            series_metadata_config,
            book_metadata_config,
            author_roles,
            artist_roles,
            cover_languages,
            links_filter,
            series_title_language,
        }
    }

    pub fn to_series_metadata(
        &self,
        manga: &MangaDexManga,
        thumbnail: Option<Image>,
        covers: &[MangaDexRelationship],
    ) -> ProviderSeriesMetadata {
        let cfg = &self.series_metadata_config;

        // 写入侧不干涉标题语言：postProcessing.seriesTitleLanguage 由后处理阶段应用。
        let (primary_title, title_type, language) = pick_title(
            &manga.attributes.title,
            &manga.attributes.alt_titles,
            None,
        );
        let title = SeriesTitle {
            name: primary_title.clone(),
            r#type: title_type,
            language: language.clone(),
        };
        let title_field = cfg.title.then_some(title.clone());

        // Kotlin: 主标题 + altTitles 合并为 titles 列表，按语言代码分类
        // （originalLanguage→Native，ja-/ko-/zh-ro→Romaji，其余→Localized）。
        let mut built_titles: Vec<SeriesTitle> = Vec::new();
        if let Some((main_lang, main_name)) = manga.attributes.title.iter().next() {
            built_titles.push(classify_title(
                main_name,
                Some(main_lang.as_str()),
                &manga.attributes.original_language,
            ));
        }
        for alt in manga.attributes.alt_titles.iter() {
            if let Some((lang, name)) = alt.iter().next() {
                built_titles.push(classify_title(
                    name,
                    Some(lang.as_str()),
                    &manga.attributes.original_language,
                ));
            }
        }
        let alt_titles: Vec<SeriesTitle> = if cfg.title {
            built_titles
        } else {
            // Kotlin seriesTitles：title 关闭时保留标题列表，仅清空 type/language
            built_titles
                .into_iter()
                .map(|t| SeriesTitle {
                    r#type: None,
                    language: None,
                    ..t
                })
                .collect()
        };

        let status = if cfg.status {
            manga.attributes.status.as_deref().and_then(map_status)
        } else {
            None
        };

        let reading_direction = cfg
            .reading_direction
            .then_some(manga.attributes.state.as_deref())
            .flatten()
            .and_then(|s| match s {
                "published" => Some(ReadingDirection::LeftToRight),
                _ => None,
            });

        // Kotlin: [publicationDemographic.lowercase()] + genre 分组标签（en 优先，回退首个译名）
        let genres: Vec<String> = if cfg.genres {
            let mut out: Vec<String> = Vec::new();
            if let Some(demo) = manga.attributes.publication_demographic.as_deref() {
                out.push(demo.to_lowercase());
            }
            out.extend(
                manga
                    .attributes
                    .tags
                    .iter()
                    .filter(|t| t.attributes.group.as_deref() == Some("genre"))
                    .filter_map(tag_name_en_or_first),
            );
            out
        } else {
            Vec::new()
        };

        // Kotlin: 仅 "theme" 分组作为标签（en 优先，回退首个译名）
        let tags: Vec<String> = if cfg.tags {
            manga
                .attributes
                .tags
                .iter()
                .filter(|t| t.attributes.group.as_deref() == Some("theme"))
                .filter_map(tag_name_en_or_first)
                .collect()
        } else {
            Vec::new()
        };

        // Kotlin: author 关系 → authorRoles 全量展开；artist 关系 → artistRoles 全量展开
        let authors = if cfg.authors {
            let mut result = Vec::new();
            for rel in manga.relationships.iter() {
                let Some(name) = rel.attributes.as_ref().and_then(|a| a.name.clone()) else {
                    continue;
                };
                if rel.r#type == "author" {
                    for role in self.author_roles.iter() {
                        result.push(Author {
                            name: name.clone(),
                            role: *role,
                        });
                    }
                } else if rel.r#type == "artist" {
                    for role in self.artist_roles.iter() {
                        result.push(Author {
                            name: name.clone(),
                            role: *role,
                        });
                    }
                }
            }
            result
        } else {
            Vec::new()
        };

        let release_date = cfg
            .release_date
            .then(|| {
                manga
                    .attributes
                    .year
                    .map(|year| crate::model::ReleaseDate::new(Some(year), None, None))
            })
            .flatten();

        let links = if cfg.links {
            build_links(manga, &self.links_filter)
        } else {
            Vec::new()
        };

        // Kotlin mangaDex 不设置 totalBookCount
        let total_book_count: Option<i32> = None;

        let metadata = SeriesMetadata {
            status,
            title: title_field,
            titles: alt_titles,
            // Kotlin: description["en"] ?: values.firstOrNull()
            summary: cfg
                .summary
                .then(|| {
                    manga
                        .attributes
                        .description
                        .get("en")
                        .cloned()
                        .or_else(|| manga.attributes.description.values().next().cloned())
                })
                .flatten(),
            publisher: None,
            alternative_publishers: Vec::new(),
            reading_direction,
            age_rating: None,
            // Kotlin mangaDex 不设置 language
            language: None,
            genres,
            tags,
            total_book_count,
            authors,
            release_date,
            links,
            score: None,
            thumbnail,
        };

        // Kotlin: books 来自封面列表，按 coverLanguages 过滤/排序，按 volume 分组取首个
        let books = if cfg.books {
            build_books_from_covers(covers, &self.cover_languages)
        } else {
            Vec::new()
        };

        ProviderSeriesMetadata {
            id: ProviderSeriesId(manga.id.clone()),
            metadata,
            books,
        }
    }

    pub fn to_series_search_result(&self, manga: &MangaDexManga) -> SeriesSearchResult {
        let (title, _, _) = pick_title(
            &manga.attributes.title,
            &manga.attributes.alt_titles,
            self.series_title_language.as_deref(),
        );
        let image_url = manga
            .relationships
            .iter()
            .find(|r| r.r#type == "cover_art")
            .and_then(|r| r.attributes.as_ref())
            .and_then(|a| a.file_name.clone())
            .map(|f| {
                format!(
                    "https://uploads.mangadex.org/covers/{}/{f}",
                    manga.id
                )
            });
        SeriesSearchResult {
            url: Some(format!("https://mangadex.org/title/{}", manga.id)),
            image_url,
            title,
            provider: CoreProviders::Mangadex.as_str().to_string(),
            result_id: manga.id.clone(),
            media_type: None,
            language: Some(manga.attributes.original_language.clone()),
            nsfw: None,
        }
    }
}

fn pick_title(
    title: &std::collections::HashMap<String, String>,
    alt_titles: &[std::collections::HashMap<String, String>],
    preference: Option<&str>,
) -> (String, Option<TitleType>, Option<String>) {
    // 标题语言偏好：主 title map 命中则直接采用；miss 再查 alt_titles（同为已返回的本地数据）；
    // 都未命中回落原有 en→ja→ja-ro→其他。仅影响显示/写入，不参与匹配。
    if let Some(pref) = preference {
        let pref_type = if pref == "ja" {
            TitleType::Native
        } else {
            TitleType::Localized
        };
        if let Some(name) = title.get(pref) {
            return (name.clone(), Some(pref_type), Some(pref.to_string()));
        }
        for alt in alt_titles {
            if let Some(name) = alt.get(pref) {
                return (name.clone(), Some(pref_type), Some(pref.to_string()));
            }
        }
    }
    if let Some(en) = title.get("en") {
        (
            en.clone(),
            Some(TitleType::Localized),
            Some("en".to_string()),
        )
    } else if let Some(ja) = title.get("ja") {
        (ja.clone(), Some(TitleType::Native), Some("ja".to_string()))
    } else if let Some(ja_ro) = title.get("ja-ro") {
        (ja_ro.clone(), Some(TitleType::Romaji), None)
    } else {
        title
            .values()
            .next()
            .map(|v| (v.clone(), None, None))
            .unwrap_or_default()
    }
}

/// 标签名：优先 "en"，缺失时回退到首个可用译名（对应 Kotlin `name["en"] ?: values.first()`）。
fn tag_name_en_or_first(tag: &MangaDexTag) -> Option<String> {
    tag.attributes
        .name
        .get("en")
        .cloned()
        .or_else(|| tag.attributes.name.values().next().cloned())
}

fn map_status(status: &str) -> Option<SeriesStatus> {
    match status.to_lowercase().as_str() {
        "completed" => Some(SeriesStatus::Ended),
        "ongoing" => Some(SeriesStatus::Ongoing),
        "hiatus" => Some(SeriesStatus::Hiatus),
        "cancelled" | "abandoned" => Some(SeriesStatus::Abandoned),
        _ => None,
    }
}

/// 对应 Kotlin titles 分类：originalLanguage→Native，ja-/ko-/zh-ro→Romaji，其余→Localized。
fn classify_title(name: &str, lang: Option<&str>, original_lang: &str) -> SeriesTitle {
    let (r#type, language) = match lang {
        Some(l) if l == original_lang => (TitleType::Native, Some(l.to_string())),
        Some(l) if l == "ja-ro" || l == "ko-ro" || l == "zh-ro" => {
            (TitleType::Romaji, Some(l.to_string()))
        }
        Some(l) => (TitleType::Localized, Some(l.to_string())),
        None => (TitleType::Localized, None),
    };
    SeriesTitle {
        name: name.to_string(),
        r#type: Some(r#type),
        language,
    }
}

/// 对应 Kotlin books：封面按 coverLanguages 过滤、排序、按 volume 分组取首个。
fn build_books_from_covers(
    covers: &[MangaDexRelationship],
    cover_languages: &[String],
) -> Vec<SeriesBook> {
    let mut filtered: Vec<&MangaDexRelationship> = covers
        .iter()
        .filter(|c| {
            c.attributes
                .as_ref()
                .and_then(|a| a.locale.as_ref())
                .map(|l| cover_languages.iter().any(|cl| cl == l))
                .unwrap_or(false)
        })
        .collect();
    filtered.sort_by(|a, b| {
        let idx = |c: &MangaDexRelationship| -> usize {
            c.attributes
                .as_ref()
                .and_then(|a| a.locale.as_ref())
                .and_then(|l| cover_languages.iter().position(|cl| cl == l))
                .unwrap_or(usize::MAX)
        };
        idx(a).cmp(&idx(b))
    });
    let mut seen = std::collections::HashSet::new();
    filtered
        .into_iter()
        .filter(|c| {
            let vol = c.attributes.as_ref().and_then(|a| a.volume.clone());
            seen.insert(vol)
        })
        .map(|c| {
            let attrs = c.attributes.as_ref().unwrap();
            SeriesBook {
                id: ProviderBookId(attrs.file_name.clone().unwrap_or_default()),
                number: attrs
                    .volume
                    .as_deref()
                    .and_then(|v| v.parse::<f64>().ok())
                    .map(crate::model::BookRange::single),
                name: attrs.volume.clone(),
                r#type: None,
                edition: None,
            }
        })
        .collect()
}

fn build_links(manga: &MangaDexManga, filter: &[crate::config::MangaDexLink]) -> Vec<WebLink> {
    use crate::config::MangaDexLink as L;
    let mut tagged: Vec<(L, WebLink)> = vec![(
        L::MangaDex,
        WebLink {
            label: "MangaDex".to_string(),
            url: format!("https://mangadex.org/title/{}", manga.id),
        },
    )];
    if let Some(map) = &manga.attributes.links {
        for (key, value) in map {
            let (lk, label, url): (L, String, String) = match key.as_str() {
                "al" => (
                    L::Anilist,
                    "AniList".into(),
                    format!("https://anilist.co/manga/{value}"),
                ),
                "ap" => (
                    L::AnimePlanet,
                    "Anime-Planet".into(),
                    format!("https://www.anime-planet.com/manga/{value}"),
                ),
                "bw" => (
                    L::BookwalkerJp,
                    "BookWalkerJp".into(),
                    format!("https://bookwalker.jp/{value}"),
                ),
                "mu" => {
                    let u = if value.parse::<i64>().is_ok() {
                        format!("https://www.mangaupdates.com/series.html?id={value}")
                    } else {
                        format!("https://www.mangaupdates.com/series/{value}")
                    };
                    (L::MangaUpdates, "MangaUpdates".into(), u)
                }
                "nu" => (
                    L::NovelUpdates,
                    "NovelUpdates".into(),
                    format!("https://www.novelupdates.com/series/{value}"),
                ),
                "kt" => (
                    L::Kitsu,
                    "Kitsu".into(),
                    format!("https://kitsu.app/manga/{value}"),
                ),
                "amz" => (L::Amazon, "Amazon".into(), value.clone()),
                "ebj" => {
                    let u = if value.parse::<i64>().is_ok() {
                        format!("https://ebookjapan.yahoo.co.jp/books/{value}")
                    } else {
                        value.clone()
                    };
                    (L::EbookJapan, "eBookJapan".into(), u)
                }
                "mal" => (
                    L::MyAnimeList,
                    "MyAnimeList".into(),
                    format!("https://myanimelist.net/manga/{value}"),
                ),
                "cdj" => (L::CdJapan, "CDJapan".into(), value.clone()),
                "raw" => (L::Raw, "Official Raw".into(), value.clone()),
                "engtl" => (L::EnglishTl, "Official English".into(), value.clone()),
                _ => continue,
            };
            tagged.push((lk, WebLink { label, url }));
        }
    }
    // Kotlin: filter 非空时仅保留过滤项；否则全部
    if filter.is_empty() {
        tagged.into_iter().map(|(_, w)| w).collect()
    } else {
        tagged
            .into_iter()
            .filter(|(k, _)| filter.contains(k))
            .map(|(_, w)| w)
            .collect()
    }
}

pub struct MangaDexMetadataProvider {
    client: MangaDexClient,
    metadata_mapper: MangaDexMetadataMapper,
    name_matcher: NameSimilarityMatcher,
    fetch_series_covers: bool,
    fetch_book_covers: bool,
    #[allow(dead_code)]
    cover_languages: Vec<String>,
}

pub fn create_provider(
    config: &MangaDexConfig,
    default_name_matcher: NameSimilarityMatcher,
    http_client: &reqwest::Client,
    series_title_language: Option<String>,
) -> Option<MangaDexMetadataProvider> {
    if !config.enabled {
        return None;
    }
    let name_matcher = config.name_matching_mode.unwrap_or(default_name_matcher);
    Some(MangaDexMetadataProvider {
        client: MangaDexClient::new(http_client.clone()),
        metadata_mapper: MangaDexMetadataMapper::new(
            config.series_metadata.clone(),
            config.book_metadata.clone(),
            config.author_roles.clone(),
            config.artist_roles.clone(),
            config.cover_languages.clone(),
            config.links.clone(),
            series_title_language,
        ),
        name_matcher,
        fetch_series_covers: config.series_metadata.thumbnail,
        fetch_book_covers: config.book_metadata.thumbnail,
        cover_languages: config.cover_languages.clone(),
    })
}

#[async_trait::async_trait]
impl MetadataProvider for MangaDexMetadataProvider {

    fn resolve_link_id(&self, query: &str) -> Option<String> {
        let re = regex::Regex::new(r"mangadex\.org/title/([0-9a-fA-F-]{8,})").ok()?;
        re.captures(query)
            .map(|c| c.get(1).unwrap().as_str().to_string())
    }
    fn provider_name(&self) -> CoreProviders {
        CoreProviders::Mangadex
    }

    fn alternative_titles_enabled(&self) -> bool {
        self.metadata_mapper.series_metadata_config.alternative_titles
    }

    async fn resolve_link_search_result(&self, query: &str) -> Option<SeriesSearchResult> {
        let id = self.resolve_link_id(query)?;
        let manga = self.client.get_series(&id).await.ok()?;
        Some(self.metadata_mapper.to_series_search_result(&manga))
    }

    async fn get_series_metadata(
        &self,
        series_id: &ProviderSeriesId,
    ) -> Result<ProviderSeriesMetadata, ProviderError> {
        let manga = self.client.get_series(&series_id.0).await?;
        let covers = self.client.get_all_covers(&series_id.0).await?;
        let thumbnail = if self.fetch_series_covers {
            if let Some(url) = self.client.get_cover_url(&manga).await {
                self.client.get_thumbnail(&url).await?
            } else {
                None
            }
        } else {
            None
        };
        Ok(self
            .metadata_mapper
            .to_series_metadata(&manga, thumbnail, &covers))
    }

    async fn get_series_cover(
        &self,
        series_id: &ProviderSeriesId,
    ) -> Result<Option<Image>, ProviderError> {
        let manga = self.client.get_series(&series_id.0).await?;
        if let Some(url) = self.client.get_cover_url(&manga).await {
            self.client.get_thumbnail(&url).await
        } else {
            Ok(None)
        }
    }

    async fn get_book_metadata(
        &self,
        series_id: &ProviderSeriesId,
        book_id: &ProviderBookId,
    ) -> Result<ProviderBookMetadata, ProviderError> {
        // Kotlin: cover = if fetchBookCovers getCover(mangaId, bookId.id); BookMetadata(thumbnail=cover)
        let cover = if self.fetch_book_covers {
            let url = format!(
                "https://uploads.mangadex.org/covers/{}/{}",
                series_id.0, book_id.0
            );
            self.client.get_thumbnail(&url).await?
        } else {
            None
        };
        Ok(ProviderBookMetadata {
            id: Some(book_id.clone()),
            series_id: Some(series_id.clone()),
            metadata: crate::model::BookMetadata {
                thumbnail: cover,
                ..Default::default()
            },
        })
    }

    async fn search_series(
        &self,
        series_name: &str,
        limit: usize,
        _media_type: Option<crate::model::MediaType>,
    ) -> Result<Vec<SeriesSearchResult>, ProviderError> {
        // Kotlin: seriesName.take(400)
        let name: String = series_name.chars().take(400).collect();
        let results = self.client.search_series(&name, limit as u32).await?;
        Ok(results
            .iter()
            .map(|m| self.metadata_mapper.to_series_search_result(m))
            .collect())
    }

    async fn match_series_metadata(
        &self,
        match_query: &MatchQuery,
    ) -> Result<Option<ProviderSeriesMetadata>, ProviderError> {
        // Kotlin: searchSeries(seriesName.take(400)) —— 默认 limit=5
        let name: String = match_query.series_name.chars().take(400).collect();
        let results = self.client.search_series(&name, 5).await?;
        let matched = results.into_iter().find(|m| {
            let mut titles: Vec<String> = m.attributes.title.values().cloned().collect();
            for alt in m.attributes.alt_titles.iter() {
                titles.extend(alt.values().cloned());
            }
            self.name_matcher.matches(
                &match_query.normalized_series_name(),
                &match_query.normalize_titles(&titles),
            )
        });
        let Some(manga) = matched else {
            return Ok(None);
        };

        let covers = self.client.get_all_covers(&manga.id).await?;
        let thumbnail = if self.fetch_series_covers {
            if let Some(url) = self.client.get_cover_url(&manga).await {
                self.client.get_thumbnail(&url).await?
            } else {
                None
            }
        } else {
            None
        };
        Ok(Some(
            self.metadata_mapper
                .to_series_metadata(&manga, thumbnail, &covers),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::pick_title;
    use std::collections::HashMap;

    fn title_map(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn pick_title_preference_hit() {
        let t = title_map(&[("en", "Berserk"), ("ja", "ベルセルク"), ("zh", "剑风传奇")]);
        let (name, ttype, lang) = pick_title(&t, &[], Some("zh"));
        assert_eq!(name, "剑风传奇");
        assert_eq!(lang.as_deref(), Some("zh"));
        assert_eq!(ttype.map(|x| format!("{x:?}")), Some("Localized".to_string()));
    }

    #[test]
    fn pick_title_preference_ja_native() {
        let t = title_map(&[("en", "Berserk"), ("ja", "ベルセルク")]);
        let (name, ttype, _) = pick_title(&t, &[], Some("ja"));
        assert_eq!(name, "ベルセルク");
        assert_eq!(ttype.map(|x| format!("{x:?}")), Some("Native".to_string()));
    }

    #[test]
    fn pick_title_preference_miss_falls_back_to_en() {
        let t = title_map(&[("en", "Berserk"), ("ja", "ベルセルク")]);
        let (name, _, _) = pick_title(&t, &[], Some("ko"));
        assert_eq!(name, "Berserk");
    }

    #[test]
    fn pick_title_preference_hits_alt_titles() {
        let t = title_map(&[("en", "Berserk")]);
        let alts = vec![title_map(&[("ja", "ベルセルク"), ("zh", "剑风传奇")])];
        let (name, _, lang) = pick_title(&t, &alts, Some("zh"));
        assert_eq!(name, "剑风传奇");
        assert_eq!(lang.as_deref(), Some("zh"));
    }

    #[test]
    fn pick_title_no_preference_keeps_legacy() {
        let t = title_map(&[("ja-ro", "Berserk (JP)"), ("en", "Berserk")]);
        let (name, _, _) = pick_title(&t, &[], None);
        assert_eq!(name, "Berserk");
    }
}
