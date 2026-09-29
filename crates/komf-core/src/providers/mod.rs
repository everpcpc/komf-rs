//! 元数据 Provider 框架 —— 对应 `snd.komf.providers` 包。
//!
//! `MetadataProvider` trait 对应 `MetadataProvider.kt`；
//! `ProvidersModule` 对应 `ProvidersModule.kt`。

pub mod anilist;
pub mod bangumi;
pub mod bangumi_archive;
pub mod bookwalker;
pub mod comicvine;
pub mod ehentai;
pub mod ehentai_archive;
pub mod mal;
pub mod mangabaka;
pub mod mangadex;
pub mod mangaupdates;
pub mod viz;
pub mod webtoons;
pub mod yenpress;

use crate::config::{MetadataProvidersConfig, ProvidersConfig};
use crate::model::{
    Image, MatchQuery, MediaType, ProviderBookId, ProviderBookMetadata, ProviderSeriesId,
    ProviderSeriesMetadata, SeriesSearchResult,
};
use crate::util::NameSimilarityMatcher;
use std::fmt;
use std::sync::Arc;

/// 用指定默认头构建带 header 的 HTTP 客户端（reqwest::Client 本身不提供 default_headers）。
pub(crate) fn client_with_default_headers(headers: reqwest::header::HeaderMap) -> reqwest::Client {
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .expect("failed to build http client")
}

/// provider 名称 —— 对应 `CoreProviders.kt` 枚举。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoreProviders {
    MangaBaka,
    BookWalker,
    Mangadex,
    MangaUpdates,
    Anilist,
    Mal,
    ComicVine,
    Bangumi,
    EHentai,
    YenPress,
    Viz,
    Webtoons,
    Kodansha,
    Nautiljon,
    Hentag,
}

impl CoreProviders {
    pub fn as_str(&self) -> &'static str {
        match self {
            CoreProviders::MangaBaka => "MANGA_BAKA",
            CoreProviders::BookWalker => "BOOK_WALKER",
            CoreProviders::Mangadex => "MANGADEX",
            CoreProviders::MangaUpdates => "MANGA_UPDATES",
            CoreProviders::Anilist => "ANILIST",
            CoreProviders::Mal => "MAL",
            CoreProviders::ComicVine => "COMIC_VINE",
            CoreProviders::Bangumi => "BANGUMI",
            CoreProviders::EHentai => "EHENTAI",
            CoreProviders::YenPress => "YEN_PRESS",
            CoreProviders::Viz => "VIZ",
            CoreProviders::Webtoons => "WEBTOONS",
            CoreProviders::Kodansha => "KODANSHA",
            CoreProviders::Nautiljon => "NAUTILJON",
            CoreProviders::Hentag => "HENTAG",
        }
    }

    pub fn from_str(name: &str) -> Option<CoreProviders> {
        match name {
            "MANGA_BAKA" => Some(CoreProviders::MangaBaka),
            "BOOK_WALKER" => Some(CoreProviders::BookWalker),
            "MANGADEX" => Some(CoreProviders::Mangadex),
            "MANGA_UPDATES" => Some(CoreProviders::MangaUpdates),
            "ANILIST" => Some(CoreProviders::Anilist),
            "MAL" => Some(CoreProviders::Mal),
            "COMIC_VINE" => Some(CoreProviders::ComicVine),
            "BANGUMI" => Some(CoreProviders::Bangumi),
            "EHENTAI" => Some(CoreProviders::EHentai),
            "YEN_PRESS" => Some(CoreProviders::YenPress),
            "VIZ" => Some(CoreProviders::Viz),
            "WEBTOONS" => Some(CoreProviders::Webtoons),
            "KODANSHA" => Some(CoreProviders::Kodansha),
            "NAUTILJON" => Some(CoreProviders::Nautiljon),
            "HENTAG" => Some(CoreProviders::Hentag),
            _ => None,
        }
    }
}

impl fmt::Display for CoreProviders {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// 元数据提供者接口 —— 对应 `MetadataProvider.kt`。
#[async_trait::async_trait]
pub trait MetadataProvider: Send + Sync {
    fn provider_name(&self) -> CoreProviders;

    async fn get_series_metadata(
        &self,
        series_id: &ProviderSeriesId,
    ) -> Result<ProviderSeriesMetadata, ProviderError>;

    async fn get_series_cover(
        &self,
        series_id: &ProviderSeriesId,
    ) -> Result<Option<Image>, ProviderError>;

    async fn get_book_metadata(
        &self,
        series_id: &ProviderSeriesId,
        book_id: &ProviderBookId,
    ) -> Result<ProviderBookMetadata, ProviderError>;

    async fn search_series(
        &self,
        series_name: &str,
        limit: usize,
        media_type: Option<MediaType>,
    ) -> Result<Vec<SeriesSearchResult>, ProviderError>;

    async fn match_series_metadata(
        &self,
        match_query: &MatchQuery,
    ) -> Result<Option<ProviderSeriesMetadata>, ProviderError>;

    /// 从搜索/匹配输入中解析本 provider 的网页链接 → 返回 series_id
    /// （Rust 扩展：输入为 provider 站点链接时直接按 id 获取，跳过站点搜索）。
    /// 默认不支持；各 provider 覆盖实现。
    fn resolve_link_id(&self, _query: &str) -> Option<String> {
        None
    }

    /// 本 provider 是否写入 alternativeTitles（`seriesMetadata.alternativeTitles`）。
    /// false 时最终写入仅保留主标题（备选不写入）；匹配与主标题语言选择仍用全量标题，不受影响。
    /// 默认 true（保持既有行为）；各 provider 返回自身 `SeriesMetadataConfig.alternative_titles`。
    fn alternative_titles_enabled(&self) -> bool {
        true
    }

    /// 链接命中时构造搜索结果（Rust 扩展：搜索框提交 provider 链接时显示用）。
    /// 默认实现：resolve_link_id → get_series_metadata → 通用模板（titles.first，
    /// 无封面 URL / mediaType）。各 provider 可重写复用其搜索结果显示逻辑
    /// （封面 URL、mediaType、标题选择、语言、nsfw）。
    async fn resolve_link_search_result(&self, query: &str) -> Option<SeriesSearchResult> {
        let id = self.resolve_link_id(query)?;
        let meta = self
            .get_series_metadata(&ProviderSeriesId(id.clone()))
            .await
            .ok()?;
        let title = meta
            .metadata
            .titles
            .first()
            .map(|t| t.name.clone())
            .or_else(|| meta.metadata.title.as_ref().map(|t| t.name.clone()))
            .unwrap_or_else(|| query.to_string());
        Some(SeriesSearchResult {
            url: Some(query.trim().to_string()),
            image_url: None,
            title,
            provider: self.provider_name().as_str().to_string(),
            result_id: id,
            media_type: None,
            language: meta.metadata.language.clone(),
            nsfw: None,
        })
    }
}

/// provider 错误。
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("{0}")]
    Message(String),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("provider {0} returned status {1}: {2}")]
    Status(CoreProviders, reqwest::StatusCode, String),
    #[error("provider {0} is not implemented in the Rust port")]
    NotImplemented(CoreProviders),
}

impl ProviderError {
    pub fn message(msg: impl Into<String>) -> Self {
        ProviderError::Message(msg.into())
    }
}

/// 注册表中单个 provider 及其优先级。
pub struct RegisteredProvider {
    pub provider: Arc<dyn MetadataProvider>,
    pub priority: i32,
}

/// 一组（默认或某个 library 的）provider 容器 —— 对应 `MetadataProvidersContainer`。
pub struct MetadataProvidersContainer {
    providers: Vec<RegisteredProvider>,
}

impl MetadataProvidersContainer {
    pub fn new(providers: Vec<RegisteredProvider>) -> Self {
        let mut providers = providers;
        providers.sort_by_key(|p| p.priority);
        Self { providers }
    }

    pub fn providers(&self) -> &[RegisteredProvider] {
        &self.providers
    }

    pub fn provider(&self, name: CoreProviders) -> Option<Arc<dyn MetadataProvider>> {
        self.providers
            .iter()
            .find(|p| p.provider.provider_name() == name)
            .map(|p| p.provider.clone())
    }
}

/// 默认 + 按 library 区分的 provider 集合 —— 对应 `MetadataProviders`。
pub struct MetadataProviders {
    default: MetadataProvidersContainer,
    library: std::collections::HashMap<String, MetadataProvidersContainer>,
}

impl MetadataProviders {
    pub fn new(default: MetadataProvidersContainer) -> Self {
        Self {
            default,
            library: Default::default(),
        }
    }

    pub fn with_library(
        mut self,
        library: std::collections::HashMap<String, MetadataProvidersContainer>,
    ) -> Self {
        self.library = library;
        self
    }

    pub fn default_providers_list(&self) -> Vec<Arc<dyn MetadataProvider>> {
        self.default
            .providers()
            .iter()
            .map(|p| p.provider.clone())
            .collect()
    }

    pub fn providers(&self, library_id: &str) -> Vec<Arc<dyn MetadataProvider>> {
        self.library
            .get(library_id)
            .unwrap_or(&self.default)
            .providers()
            .iter()
            .map(|p| p.provider.clone())
            .collect()
    }

    pub fn provider(
        &self,
        library_id: &str,
        name: CoreProviders,
    ) -> Option<Arc<dyn MetadataProvider>> {
        self.library
            .get(library_id)
            .unwrap_or(&self.default)
            .provider(name)
    }
}

/// Provider 模块装配 —— 对应 `ProvidersModule.kt`。
///
/// `database_work_dir` 对应 Kotlin `CoreModule` 的 workDir：其下 `mangabaka/`、
/// `bookwalker/` 子目录存放 SQLite 数据库（由下载器维护）。数据库缺失时
/// BookWalker 不注册、MangaBaka DATABASE 模式禁用（与 Kotlin 行为一致）。
pub struct ProvidersModule {
    pub metadata_providers: MetadataProviders,
    /// OAuth 管理器（AniList/MAL/Bangumi 登录态；None = 未初始化）。
    pub oauth_manager: Option<Arc<crate::oauth::OAuthManager>>,
}

impl ProvidersModule {
    pub fn new(
        config: &MetadataProvidersConfig,
        http_client: reqwest::Client,
        database_work_dir: Option<&std::path::Path>,
    ) -> Self {
        Self::with_oauth(config, http_client, database_work_dir, None, None)
    }

    /// 带 OAuth 的构造：`oauth_manager` 由 app 层创建（共享 http client 与 work_dir）。
    pub fn with_oauth(
        config: &MetadataProvidersConfig,
        http_client: reqwest::Client,
        database_work_dir: Option<&std::path::Path>,
        oauth_manager: Option<Arc<crate::oauth::OAuthManager>>,
        series_title_language: Option<String>,
    ) -> Self {
        let default_name_matcher = config.name_matching_mode;

        let default_providers = create_metadata_providers(
            &config.default_providers,
            default_name_matcher,
            config,
            &http_client,
            database_work_dir,
            oauth_manager.clone(),
            series_title_language.clone(),
        );
        let library_providers = config
            .library_providers
            .iter()
            .map(|(library_id, library_config)| {
                (
                    library_id.clone(),
                    create_metadata_providers(
                        library_config,
                        default_name_matcher,
                        config,
                        &http_client,
                        database_work_dir,
                        oauth_manager.clone(),
                        series_title_language.clone(),
                    ),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();

        Self {
            metadata_providers: MetadataProviders::new(default_providers)
                .with_library(library_providers),
            oauth_manager,
        }
    }
}

fn create_metadata_providers(
    config: &ProvidersConfig,
    default_name_matcher: NameSimilarityMatcher,
    global: &MetadataProvidersConfig,
    http_client: &reqwest::Client,
    database_work_dir: Option<&std::path::Path>,
    oauth_manager: Option<Arc<crate::oauth::OAuthManager>>,
    series_title_language: Option<String>,
) -> MetadataProvidersContainer {
    let mut providers: Vec<RegisteredProvider> = Vec::new();

    // 对应 Kotlin CoreModule：mangabaka/mangabaka.sqlite、bookwalker/bkwk-db.sqlite
    let manga_baka_db = database_work_dir.map(|d| d.join("mangabaka").join("mangabaka.sqlite"));
    let book_walker_db = database_work_dir.map(|d| d.join("bookwalker").join("bkwk-db.sqlite"));

    if let Some(p) =
        mangaupdates::create_provider(&config.manga_updates, default_name_matcher, http_client)
    {
        providers.push(RegisteredProvider {
            provider: Arc::new(p),
            priority: config.manga_updates.priority,
        });
    }
    if let Some(p) = mal::create_provider(
        &config.mal,
        global.mal_client_id.as_deref(),
        default_name_matcher,
        http_client,
        oauth_manager.clone(),
    ) {
        providers.push(RegisteredProvider {
            provider: Arc::new(p),
            priority: config.mal.priority,
        });
    }
    if let Some(p) = anilist::create_provider(
        &config.ani_list,
        default_name_matcher,
        http_client,
        oauth_manager.clone(),
    ) {
        providers.push(RegisteredProvider {
            provider: Arc::new(p),
            priority: config.ani_list.priority,
        });
    }
    if let Some(p) = mangadex::create_provider(
        &config.manga_dex,
        default_name_matcher,
        http_client,
        series_title_language.clone(),
    ) {
        providers.push(RegisteredProvider {
            provider: Arc::new(p),
            priority: config.manga_dex.priority,
        });
    }
    if let Some(p) = bangumi::create_provider(
        &config.bangumi,
        default_name_matcher,
        global.bangumi_token.as_deref(),
        http_client,
        database_work_dir,
        oauth_manager.clone(),
    ) {
        providers.push(RegisteredProvider {
            provider: Arc::new(p),
            priority: config.bangumi.provider.priority,
        });
    }
    if let Some(p) = comicvine::create_provider(
        &config.comic_vine,
        global.comic_vine_api_key.as_deref(),
        global.comic_vine_search_limit,
        global.comic_vine_issue_name.as_deref(),
        global.comic_vine_id_format.as_deref(),
        default_name_matcher,
        http_client,
    ) {
        providers.push(RegisteredProvider {
            provider: Arc::new(p),
            priority: config.comic_vine.priority,
        });
    }

    // MangaBaka：API 或 DATABASE（本地 SQLite）数据源；BookWalker：本地 SQLite 数据库。
    if let Some(p) = mangabaka::create_provider(
        &config.manga_baka,
        default_name_matcher,
        http_client,
        manga_baka_db.as_deref(),
        series_title_language.clone(),
    ) {
        providers.push(RegisteredProvider {
            provider: Arc::new(p),
            priority: config.manga_baka.priority,
        });
    }
    if let Some(p) = bookwalker::create_provider(
        &config.book_walker,
        book_walker_db.as_deref(),
        default_name_matcher,
        http_client,
    ) {
        providers.push(RegisteredProvider {
            provider: Arc::new(p),
            priority: config.book_walker.priority,
        });
    }
    if let Some(p) = yenpress::create_provider(&config.yen_press, default_name_matcher, http_client)
    {
        providers.push(RegisteredProvider {
            provider: Arc::new(p),
            priority: config.yen_press.priority,
        });
    }
    if let Some(p) = viz::create_provider(&config.viz, default_name_matcher, http_client) {
        providers.push(RegisteredProvider {
            provider: Arc::new(p),
            priority: config.viz.priority,
        });
    }
    if let Some(p) = webtoons::create_provider(&config.webtoons, default_name_matcher, http_client)
    {
        providers.push(RegisteredProvider {
            provider: Arc::new(p),
            priority: config.webtoons.priority,
        });
    }
    // EHentai —— 对应 Kotlin PR #284 createEHentaiMetadataProvider
    if let Some(p) = ehentai::create_provider(
        &config.e_hentai,
        default_name_matcher,
        http_client,
        database_work_dir,
    ) {
        providers.push(RegisteredProvider {
            provider: Arc::new(p),
            priority: config.e_hentai.priority,
        });
    }

    MetadataProvidersContainer::new(providers)
}

/// 未实现的 provider 桩。
pub struct UnimplementedProvider {
    name: CoreProviders,
    enabled: bool,
}

#[async_trait::async_trait]
impl MetadataProvider for UnimplementedProvider {
    fn provider_name(&self) -> CoreProviders {
        self.name
    }

    async fn get_series_metadata(
        &self,
        _id: &ProviderSeriesId,
    ) -> Result<ProviderSeriesMetadata, ProviderError> {
        Err(ProviderError::NotImplemented(self.name))
    }

    async fn get_series_cover(
        &self,
        _id: &ProviderSeriesId,
    ) -> Result<Option<Image>, ProviderError> {
        Err(ProviderError::NotImplemented(self.name))
    }

    async fn get_book_metadata(
        &self,
        _series_id: &ProviderSeriesId,
        _book_id: &ProviderBookId,
    ) -> Result<ProviderBookMetadata, ProviderError> {
        Err(ProviderError::NotImplemented(self.name))
    }

    async fn search_series(
        &self,
        _series_name: &str,
        _limit: usize,
        _media_type: Option<crate::model::MediaType>,
    ) -> Result<Vec<SeriesSearchResult>, ProviderError> {
        if !self.enabled {
            return Ok(Vec::new());
        }
        Err(ProviderError::NotImplemented(self.name))
    }

    async fn match_series_metadata(
        &self,
        _match_query: &MatchQuery,
    ) -> Result<Option<ProviderSeriesMetadata>, ProviderError> {
        Err(ProviderError::NotImplemented(self.name))
    }
}
