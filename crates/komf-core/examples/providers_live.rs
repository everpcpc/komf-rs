//! 真实联调：逐个验证 Rust 版各元数据提供商是否正常工作。
//!
//! 用法：`cargo run -p komf-core --example providers_live`
//! 可选 env：`KOMF_MAL_CLIENT_ID`、`KOMF_COMIC_VINE_API_KEY`、`KOMF_BOOKWALKER_DB`
//! （未设置对应 provider 标记 SKIP）。
use komf_core::config::{
    AniListConfig, BookMetadataConfig, MangaDexConfig, ProviderConfig, SeriesMetadataConfig,
};
use komf_core::providers::MetadataProvider;
use komf_core::util::NameSimilarityMatcher;

const SEARCH: &str = "One Piece";

fn cfg() -> SeriesMetadataConfig {
    SeriesMetadataConfig::default()
}

fn book_cfg() -> BookMetadataConfig {
    BookMetadataConfig::default()
}

#[tokio::main]
async fn main() {
    let http = reqwest::Client::builder()
        .user_agent("dyphire/komf-rs (https://github.com/dyphire/komf-rs)")
        .build()
        .unwrap();
    let matcher = NameSimilarityMatcher::default();

    // 1) MangaUpdates（公开 API，无需 key）
    {
        let p = komf_core::providers::mangaupdates::create_provider(
            &ProviderConfig {
                priority: 1,
                enabled: true,
                ..Default::default()
            },
            matcher,
            &http,
        );
        run(&p, "MangaUpdates").await;
    }

    // 2) AniList（公开 GraphQL，无需 key）
    {
        let p = komf_core::providers::anilist::create_provider(
            &AniListConfig {
                priority: 2,
                enabled: true,
                series_metadata: cfg(),
                ..Default::default()
            },
            matcher,
            &http,
            None,
        );
        run(&p, "AniList").await;
    }

    // 3) MangaDex（公开 API v5，无需 key）
    {
        let p = komf_core::providers::mangadex::create_provider(
            &MangaDexConfig {
                priority: 3,
                enabled: true,
                series_metadata: cfg(),
                book_metadata: book_cfg(),
                ..Default::default()
            },
            matcher,
            &http,
            None,
        );
        run(&p, "MangaDex").await;
    }

    // 4) Bangumi（公开 API）
    {
        let p = komf_core::providers::bangumi::create_provider(
            &komf_core::config::BangumiConfig {
                provider: ProviderConfig {
                    priority: 4,
                    enabled: true,
                    ..Default::default()
                },
                archive: komf_core::config::BangumiArchiveConfig::default(),
                series_title_language: None,
            },
            matcher,
            None,
            &http,
            None,
            None,
        );
        run(&p, "Bangumi").await;
    }

    // 5) MAL（需 X-MAL-CLIENT-ID）
    {
        let key = std::env::var("KOMF_MAL_CLIENT_ID").ok();
        match &key {
            Some(k) if !k.is_empty() => {
                let p = komf_core::providers::mal::create_provider(
                    &ProviderConfig {
                        priority: 5,
                        enabled: true,
                        ..Default::default()
                    },
                    Some(k),
                    matcher,
                    &http,
                    None,
                );
                run(&p, "MAL").await;
            }
            _ => println!("MAL        : SKIP (KOMF_MAL_CLIENT_ID 未设置)"),
        }
    }

    // 6) ComicVine（需 apiKey）
    {
        let key = std::env::var("KOMF_COMIC_VINE_API_KEY").ok();
        match &key {
            Some(k) if !k.is_empty() => {
                let p = komf_core::providers::comicvine::create_provider(
                    &ProviderConfig {
                        priority: 6,
                        enabled: true,
                        ..Default::default()
                    },
                    Some(k),
                    None,
                    None,
                    None,
                    matcher,
                    &http,
                );
                run(&p, "ComicVine").await;
            }
            _ => println!("ComicVine  : SKIP (KOMF_COMIC_VINE_API_KEY 未设置)"),
        }
    }

    // 7) MangaBaka API 模式
    {
        let p = komf_core::providers::mangabaka::create_provider(
            &komf_core::config::MangaBakaConfig {
                priority: 7,
                enabled: true,
                ..Default::default()
            },
            matcher,
            &http,
            None,
            None,
        );
        run(&p, "MangaBaka(API)").await;
    }

    // 8) BookWalker（需本地 SQLite，env KOMF_BOOKWALKER_DB）
    {
        let db = std::env::var("KOMF_BOOKWALKER_DB").ok();
        match &db {
            Some(path) if std::path::Path::new(path).exists() => {
                let p = komf_core::providers::bookwalker::create_provider(
                    &ProviderConfig {
                        priority: 8,
                        enabled: true,
                        ..Default::default()
                    },
                    Some(std::path::Path::new(path)),
                    matcher,
                    &http,
                );
                run(&p, "BookWalker").await;
            }
            _ => println!("BookWalker : SKIP (KOMF_BOOKWALKER_DB 指向 bkwk-db.sqlite)"),
        }
    }
}

async fn run(p: &Option<impl MetadataProvider>, name: &str) {
    let Some(p) = p else {
        println!("{name:<11}: CREATE_FAILED");
        return;
    };
    match p.search_series(SEARCH, 5, None).await {
        Ok(hits) => {
            println!("{name:<11}: OK {} hits", hits.len());
            for h in hits.iter().take(2) {
                println!("           - {} | {}", h.result_id, h.title);
            }
            if let Some(first) = hits.first() {
                let id = komf_core::model::ProviderSeriesId(first.result_id.clone());
                match p.get_series_metadata(&id).await {
                    Ok(meta) => println!(
                        "           metadata: title={:?} desc_len={} authors={}",
                        meta.metadata.titles.first().map(|t| t.name.clone()),
                        meta.metadata.summary.as_ref().map(|s| s.len()).unwrap_or(0),
                        meta.metadata.authors.len()
                    ),
                    Err(e) => println!("           metadata ERR: {e}"),
                }
            }
            // match 路径：名字匹配应命中第一个结果
            match p
                .match_series_metadata(&komf_core::model::MatchQuery {
                    series_name: SEARCH.to_string(),
                    start_year: None,
                    book_qualifier: None,
                    series_folder: None,
                    normalization_regex: None,
                    media_type: None,
                    chinese: None,
                    oneshot: false,
                    book_file_name: None,
                })
                .await
            {
                Ok(Some(m)) => println!(
                    "           match: OK title={:?}",
                    m.metadata.titles.first().map(|t| t.name.clone())
                ),
                Ok(None) => println!("           match: no match"),
                Err(e) => println!("           match ERR: {e}"),
            }
        }
        Err(e) => println!("{name:<11}: ERR {e}"),
    }
}
