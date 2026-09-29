//! 应用上下文 —— 对应 `AppContext.kt`。
use crate::config::{ConfigLoader, ConfigWriter};
use crate::routes::{AppState, SharedState};
use komf_core::providers::bookwalker::BookWalkerDbDownloader;
use komf_core::providers::mangabaka::{MangaBakaDbDownloader, MangaBakaDbRepository};
use komf_core::providers::ProvidersModule;
use komf_mediaserver::MediaServerModule;
use komf_notifications::NotificationsModule;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

pub static APP_CONTEXT: OnceLock<Arc<AppContext>> = OnceLock::new();

/// 应用上下文：持有配置、HTTP 客户端与可整体替换的模块状态。
pub struct AppContext {
    pub state: SharedState,
    config_path: Option<PathBuf>,
    http_client: reqwest::Client,
    /// OAuth 管理器（AniList/MAL/Bangumi 登录态；热重载复用同一实例，token 不丢）。
    pub oauth_manager: Arc<komf_core::oauth::OAuthManager>,
}

impl AppContext {
    pub fn new(config_path: Option<PathBuf>) -> Self {
        let config = ConfigLoader::load(config_path.as_deref());
        init_logging(&config.log_level, config_path.as_deref());

        let http_client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            // Rust 扩展：总超时 60s（Kotlin ktor 无总超时），防止单个 provider
            // 请求挂起导致 Auto-Identify Library / 事件监听整条链卡死。
            .timeout(std::time::Duration::from_secs(60))
            // 对应 Kotlin 全局 ktor UserAgent 插件（MangaDex 等 API 强制要求非默认 UA）
            .user_agent("dyphire/komf-rs (https://github.com/dyphire/komf-rs)")
            .build()
            .expect("failed to build http client");

        let work_dir = work_dir_from(config_path.as_deref());
        let oauth_manager =
            komf_core::oauth::OAuthManager::new(Some(&work_dir), http_client.clone());
        let state = build_state(&config, &http_client, config_path.as_deref(), oauth_manager.clone());

        Self {
            state: Arc::new(std::sync::RwLock::new(state)),
            config_path,
            http_client,
            oauth_manager,
        }
    }

    /// 热重载：以新配置重建所有模块并整体替换状态。
    pub fn reload(&self, new_config: crate::config::AppConfig) -> anyhow::Result<()> {
        tracing::info!("Reconfiguring application state");
        let new_state = build_state(
            &new_config,
            &self.http_client,
            self.config_path.as_deref(),
            self.oauth_manager.clone(),
        );
        *self.state.write().unwrap() = new_state;
        if let Some(path) = &self.config_path {
            ConfigWriter::write_config(&new_config, path).ok();
        } else {
            ConfigWriter::write_config_to_default_path(&new_config).ok();
        }
        Ok(())
    }
}

/// workDir 语义：configPath 为目录 → 本身；文件 → 父目录；None → 当前目录。
fn work_dir_from(config_path: Option<&std::path::Path>) -> PathBuf {
    match config_path {
        Some(path) if path.is_dir() => path.to_path_buf(),
        Some(path) => path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".")),
        None => PathBuf::from("."),
    }
}

fn build_state(
    config: &crate::config::AppConfig,
    http_client: &reqwest::Client,
    config_path: Option<&std::path::Path>,
    oauth_manager: Arc<komf_core::oauth::OAuthManager>,
) -> AppState {
    // 对应 Kotlin `AppContext`：workDir = configDir（数据库下载器工作目录）
    let work_dir = work_dir_from(config_path);
    let db_work_dir = work_dir.join("mangabaka");

    // MangaDex/MangaBaka 标题语言偏好复用全局 postProcessing.seriesTitleLanguage
    let series_title_language = config
        .komga
        .metadata_update
        .default
        .post_processing
        .series_title_language
        .clone();
    let providers_module = ProvidersModule::with_oauth(
        &config.metadata_providers,
        http_client.clone(),
        Some(&work_dir),
        Some(oauth_manager.clone()),
        series_title_language,
    );
    let notifications_module = NotificationsModule::new(&config.notifications, http_client.clone());
    let media_server_module = MediaServerModule::new(
        &config.komga,
        &config.kavita,
        &config.stump,
        &config.database,
        Arc::new(providers_module.metadata_providers),
        http_client.clone(),
        notifications_module.discord_webhook_service.clone(),
        notifications_module.apprise_service.clone(),
        // mylar ${configDir} 占位符基准（=配置目录，work_dir 语义：目录/文件父目录/cwd）
        Some(work_dir.clone()),
    );
    let manga_baka_db_downloader = Arc::new(MangaBakaDbDownloader::new(
        db_work_dir.clone(),
        http_client.clone(),
    ));
    // MangaBaka 管理 API 仓储：数据库文件存在时启用（link/unlink/tags/系列详情）。
    let manga_baka_repository = {
        let database_file = db_work_dir.join("mangabaka.sqlite");
        if database_file.exists() {
            Some(Arc::new(MangaBakaDbRepository::new(database_file)))
        } else {
            None
        }
    };
    let book_walker_db_downloader = Arc::new(BookWalkerDbDownloader::new(
        work_dir.join("bookwalker"),
        http_client.clone(),
    ));
    let tracker_services = Arc::new(komf_core::trackers::TrackerServices::new(
        http_client.clone(),
        Some(oauth_manager.clone()),
    ));
    AppState::from_modules(
        config.clone(),
        media_server_module,
        notifications_module,
        manga_baka_db_downloader,
        manga_baka_repository,
        book_walker_db_downloader,
        oauth_manager,
        tracker_services,
    )
}

/// 供路由层调用的热重载入口。
pub async fn reload_with_config(config: crate::config::AppConfig) -> anyhow::Result<()> {
    let context = APP_CONTEXT
        .get()
        .ok_or_else(|| anyhow::anyhow!("application context not initialized"))?;
    context.reload(config)
}

/// Windows 下将控制台代码页切换为 UTF-8（65001），保证中文日志在交互
/// 终端正常显示；stdout 始终输出 UTF-8 字节，重定向到文件时此调用无副作用。
#[cfg(windows)]
fn set_console_utf8() {
    unsafe {
        #[link(name = "kernel32")]
        extern "system" {
            fn SetConsoleOutputCP(w_code_page_id: u32) -> i32;
            fn SetConsoleCP(w_code_page_id: u32) -> i32;
        }
        const CP_UTF8: u32 = 65001;
        SetConsoleOutputCP(CP_UTF8);
        SetConsoleCP(CP_UTF8);
    }
}

#[cfg(not(windows))]
fn set_console_utf8() {}

/// 初始化全局日志（重复调用被忽略；热重载改变日志级别需重启生效）。
///
/// Rust 扩展：无配置项，控制台与日志文件双写。控制台保留 ANSI 彩色，
/// 日志文件不带 ANSI 转义码；两者同受配置 logLevel（或 RUST_LOG）过滤。
/// 日志统一放入 `work_dir/logs` 子目录（configPath 目录 / 文件父目录 /
/// 当前目录），当日写入 `komf.log`，跨天把前一日备份为
/// `komf.YYYY-MM-DD.log`（每日轮转）。
pub fn init_logging(level: &str, config_path: Option<&std::path::Path>) {
    // Windows 交互终端默认代码页（GBK）会把 UTF-8 中文显示成乱码，
    // 启动时切换为 UTF-8（重定向到文件时无副作用）。
    set_console_utf8();
    let filter = match level.to_uppercase().as_str() {
        "TRACE" => "trace",
        "DEBUG" => "debug",
        "WARN" => "warn",
        "ERROR" => "error",
        _ => "info",
    };
    // work_dir 语义与 build_state 一致；日志统一放入 work_dir/logs 子目录
    let log_dir: PathBuf = match config_path {
        Some(path) if path.is_dir() => path.to_path_buf(),
        Some(path) => path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".")),
        None => PathBuf::from("."),
    }
    .join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let file_writer: LogWriter = LogWriter(Arc::new(std::sync::Mutex::new(DailyLogFile::new(
        log_dir,
    ))));

    // 同一过滤器（配置 logLevel / RUST_LOG），控制台与文件分别输出。
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter));

    // 控制台 layer：保留 ANSI 彩色。
    let console_layer = tracing_subscriber::fmt::layer().with_filter(filter.clone());

    // 文件 layer：无 ANSI 转义码。
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false)
        .with_filter(filter);

    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::Layer as _; // with_filter 来自 Layer trait
    let _ = tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .try_init();
}

/// 日志文件 writer（MakeWriter）：供文件 layer 使用。
#[derive(Clone)]
struct LogWriter(Arc<std::sync::Mutex<DailyLogFile>>);

/// 每次 make_writer 克隆 Arc，写入时短暂持有锁。
struct FileWriter(Arc<std::sync::Mutex<DailyLogFile>>);

impl std::io::Write for FileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .flush()
    }
}

impl<'a> tracing_subscriber::fmt::writer::MakeWriter<'a> for LogWriter {
    type Writer = FileWriter;
    fn make_writer(&'a self) -> Self::Writer {
        FileWriter(self.0.clone())
    }
}

/// 备份日志保留天数（暂定 30 天）：跨天轮转时清理更早的备份文件。
/// 后续如需配置化，把此常量改为配置项即可。
const LOG_RETENTION_DAYS: i64 = 30;

/// 每日轮转日志文件：当日写 `komf.log`，跨天首次写入时把前一日重命名为
/// `komf.YYYY-MM-DD.log` 后新建当日文件。首次启动时若存在跨天残留的
/// `komf.log`，按文件修改时间备份，避免旧日志被吞并。
struct DailyLogFile {
    dir: PathBuf,
    date: String,
    file: Option<std::fs::File>,
}

impl DailyLogFile {
    fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            date: String::new(),
            file: None,
        }
    }

    fn rotate_if_needed(&mut self) -> std::io::Result<()> {
        use chrono::Datelike;
        let now = chrono::Local::now();
        let today = format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day());
        if self.date == today {
            return Ok(());
        }
        if let Some(file) = self.file.take() {
            drop(file); // 先关闭旧文件再重命名（Windows 上必需）
        }
        let current = self.dir.join("komf.log");
        if current.exists() {
            // 归档日期：跨天滚动用 self.date；首次启动用文件修改时间（若跨天）。
            let mut archive_date = self.date.clone();
            if archive_date.is_empty() {
                if let Ok(meta) = current.metadata() {
                    if let Ok(modified) = meta.modified() {
                        let modified: chrono::DateTime<chrono::Local> = modified.into();
                        let d = format!(
                            "{:04}-{:02}-{:02}",
                            modified.year(),
                            modified.month(),
                            modified.day()
                        );
                        if d != today {
                            archive_date = d;
                        }
                    }
                }
            }
            if !archive_date.is_empty() {
                let backup = self.dir.join(format!("komf.{archive_date}.log"));
                let _ = std::fs::rename(&current, &backup); // 备份已存在（同日重启）则忽略
            }
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&current)?;
        self.file = Some(file);
        self.date = today;
        self.prune_old_backups();
        Ok(())
    }

    /// 删除超过保留期（默认 30 天）的备份日志，仅处理 `komf.YYYY-MM-DD.log`，
    /// 当日 `komf.log` 不在此列；解析失败或删除失败一律静默忽略。
    fn prune_old_backups(&mut self) {
        let cutoff = chrono::Local::now() - chrono::Duration::days(LOG_RETENTION_DAYS);
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(date_str) = name.strip_prefix("komf.").and_then(|s| s.strip_suffix(".log")) else {
                continue;
            };
            let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") else {
                continue;
            };
            if date < cutoff.date_naive() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

impl std::io::Write for DailyLogFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.rotate_if_needed()?;
        self.file
            .as_mut()
            .ok_or_else(|| std::io::Error::other("log file not open"))?
            .write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(file) = self.file.as_mut() {
            file.flush()?;
        }
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// 每日轮转：跨天首次写入把前一日 komf.log 备份为 komf.YYYY-MM-DD.log，
    /// 同日继续写入不重复备份。
    #[test]
    fn daily_log_rotates_and_backs_up() {
        use chrono::Datelike;
        let dir = std::env::temp_dir().join(format!("komf-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let yesterday_dt = chrono::Local::now() - chrono::Duration::days(1);
        let yesterday = format!(
            "{:04}-{:02}-{:02}",
            yesterday_dt.year(),
            yesterday_dt.month(),
            yesterday_dt.day()
        );
        let today = format!(
            "{:04}-{:02}-{:02}",
            chrono::Local::now().year(),
            chrono::Local::now().month(),
            chrono::Local::now().day()
        );

        // 模拟昨天已有日志文件 + 进程内 date 停留在昨天
        std::fs::write(dir.join("komf.log"), b"old line\n").unwrap();
        let mut log = DailyLogFile::new(dir.clone());
        log.date = yesterday.clone();

        use std::io::Write as _;
        log.write_all(b"new line\n").unwrap();
        log.flush().unwrap();

        // 旧日志被备份，新日志写入当日文件
        assert!(dir.join(format!("komf.{yesterday}.log")).exists(), "backup missing");
        let cur = std::fs::read_to_string(dir.join("komf.log")).unwrap();
        assert!(cur.contains("new line"), "current file missing new content");
        assert!(!cur.contains("old line"), "old content leaked into current file");

        // 同日再次写入：不重复备份
        log.write_all(b"more\n").unwrap();
        log.flush().unwrap();
        assert!(!dir.join(format!("komf.{today}.log")).exists(), "unexpected same-day backup");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 首次启动残留跨天旧文件：按文件修改时间归档，不吞并旧日志。
    #[test]
    fn daily_log_first_start_archives_stale_file() {
        let dir = std::env::temp_dir().join(format!("komf-log-test2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 制造一个昨天修改的 komf.log
        std::fs::write(dir.join("komf.log"), b"stale\n").unwrap();
        let yesterday_dt = chrono::Local::now() - chrono::Duration::days(1);
        let stale: chrono::DateTime<chrono::Local> = yesterday_dt.into();
        let filetime = std::fs::FileTimes::new()
            .set_modified(stale.into());
        std::fs::OpenOptions::new()
            .write(true)
            .open(dir.join("komf.log"))
            .unwrap()
            .set_times(filetime)
            .unwrap();

        let mut log = DailyLogFile::new(dir.clone()); // date 为空（首次）
        use std::io::Write as _;
        log.write_all(b"fresh\n").unwrap();
        log.flush().unwrap();

        let backups: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with("komf.") && n.ends_with(".log") && n != "komf.log")
            .collect();
        assert_eq!(backups.len(), 1, "expected one backup, got {backups:?}");
        let cur = std::fs::read_to_string(dir.join("komf.log")).unwrap();
        assert!(cur.contains("fresh"));
        assert!(!cur.contains("stale"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 过期日志清理：跨天轮转时删除超过 30 天的备份，近期备份与当日文件保留。
    #[test]
    fn prune_removes_old_backups_only() {
        let dir = std::env::temp_dir().join(format!("komf-log-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 构造：旧备份（60 天前）、近期备份（5 天前）、当日文件
        let now = chrono::Local::now();
        let old_date = (now - chrono::Duration::days(60)).format("%Y-%m-%d").to_string();
        let recent_date = (now - chrono::Duration::days(5)).format("%Y-%m-%d").to_string();
        let yesterday = (now - chrono::Duration::days(1)).format("%Y-%m-%d").to_string();
        std::fs::write(dir.join(format!("komf.{old_date}.log")), b"old\n").unwrap();
        std::fs::write(dir.join(format!("komf.{recent_date}.log")), b"recent\n").unwrap();
        std::fs::write(dir.join("komf.log"), b"today\n").unwrap();

        // 进程内 date 停在昨天，跨天首次写入触发轮转 + 清理
        let mut log = DailyLogFile::new(dir.clone());
        log.date = yesterday.clone();
        use std::io::Write as _;
        log.write_all(b"new\n").unwrap();
        log.flush().unwrap();

        assert!(!dir.join(format!("komf.{old_date}.log")).exists(), "old backup not pruned");
        assert!(dir.join(format!("komf.{recent_date}.log")).exists(), "recent backup lost");
        assert!(dir.join("komf.log").exists(), "current log lost");
        assert!(dir.join(format!("komf.{yesterday}.log")).exists(), "yesterday backup missing");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
