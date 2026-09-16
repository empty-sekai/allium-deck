//! Startup configuration: command-line flags with environment-variable fallbacks.
//!
//! Every flag `--some-name` also reads `ALLIUM_DECK_SOME_NAME`; the flag wins when
//! both are present. Parsing is hand-rolled to keep the dependency set small.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;

/// Default deadline the search is allowed to spend on one request.
///
/// The engine itself accepts up to 300_000 ms, which is long enough for a single
/// request to hold a worker for five minutes. The service clamps to this instead;
/// raise it explicitly with `--max-search-timeout-ms` to allow the longer budget.
const DEFAULT_MAX_SEARCH_TIMEOUT_MS: u64 = 2_000;
const DEFAULT_QUEUE_TIMEOUT_MS: u64 = 1_000;
const DEFAULT_MAX_LIMIT: usize = 30;
const DEFAULT_MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_BIND: &str = "0.0.0.0:8080";

/// One region's masterdata on disk.
#[derive(Debug, Clone)]
pub struct RegionSource {
    /// Region tag used in request bodies and in `/v1/regions`.
    pub name: String,
    /// Directory holding the flat masterdata `*.json` tables.
    pub masterdata_dir: PathBuf,
    /// `music_metas.json` for this region.
    pub music_metas: PathBuf,
}

/// Resolved service configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub regions: Vec<RegionSource>,
    pub default_region: String,
    pub workers: usize,
    pub max_queue: usize,
    pub queue_timeout_ms: u64,
    pub max_search_timeout_ms: u64,
    pub max_limit: usize,
    pub max_body_bytes: usize,
    pub admin_token: Option<String>,
    pub log_json: bool,
}

/// What [`parse`] produced: a runnable configuration, or text to print before exiting.
pub enum Parsed {
    Run(Box<Config>),
    Help(String),
    Version(String),
}

/// Options accepted on the command line, excluding the repeatable region pairs.
const SINGLE_FLAGS: [&str; 10] = [
    "bind",
    "default-region",
    "workers",
    "max-queue",
    "queue-timeout-ms",
    "max-search-timeout-ms",
    "max-limit",
    "max-body-bytes",
    "admin-token",
    "log-format",
];

pub const HELP: &str = "allium-deck-server - HTTP service for the allium-deck recommendation engine

USAGE:
    allium-deck-server --masterdata <region>=<dir> --music-metas <region>=<file> [OPTIONS]

REQUIRED:
    --masterdata <region>=<dir>    Masterdata directory for a region. Repeatable.
    --music-metas <region>=<file>  music_metas.json for a region. Repeatable.

OPTIONS:
    --bind <addr>                  Listen address. Default: 0.0.0.0:8080
    --default-region <name>        Region used when a request omits one.
                                   Default: the first --masterdata given.
    --workers <n>                  Search threads. Default: available parallelism.
    --max-queue <n>                Queued requests before 503. Default: workers * 8.
    --queue-timeout-ms <ms>        Wait budget in the queue before 504. Default: 1000.
    --max-search-timeout-ms <ms>   Upper bound for a request's timeoutMs. Default: 2000.
    --max-limit <n>                Upper bound for a request's limit. Default: 30.
    --max-body-bytes <n>           Request body cap. Default: 8388608.
    --admin-token <token>          Enables POST /admin/reload with this bearer token.
    --log-format <text|json>       Log encoding. Default: text.
    -h, --help                     Print this help.
    -V, --version                  Print the version.

Every option also reads an environment variable: --max-queue is ALLIUM_DECK_MAX_QUEUE,
and so on. Repeatable options take a comma-separated list there, for example
ALLIUM_DECK_MASTERDATA=cn=/data/cn,jp=/data/jp
";

/// Parses `args` (without the executable name) against the process environment.
pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut masterdata: Vec<(String, String)> = Vec::new();
    let mut music_metas: Vec<(String, String)> = Vec::new();
    let mut single: BTreeMap<String, String> = BTreeMap::new();

    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "-h" | "--help" => return Ok(Parsed::Help(HELP.to_string())),
            "-V" | "--version" => {
                return Ok(Parsed::Version(format!(
                    "allium-deck-server {}",
                    env!("CARGO_PKG_VERSION")
                )));
            }
            "--masterdata" => masterdata.push(split_pair(&take(args, &mut index, arg)?)?),
            "--music-metas" => music_metas.push(split_pair(&take(args, &mut index, arg)?)?),
            other => {
                let name = other
                    .strip_prefix("--")
                    .filter(|name| SINGLE_FLAGS.contains(name))
                    .ok_or_else(|| format!("unknown option {other}"))?
                    .to_string();
                let value = take(args, &mut index, other)?;
                single.insert(name, value);
            }
        }
        index += 1;
    }

    if masterdata.is_empty() {
        masterdata = env_pairs("ALLIUM_DECK_MASTERDATA")?;
    }
    if music_metas.is_empty() {
        music_metas = env_pairs("ALLIUM_DECK_MUSIC_METAS")?;
    }
    if masterdata.is_empty() {
        return Err("no masterdata given: pass --masterdata <region>=<dir>".to_string());
    }

    let music_by_region: BTreeMap<String, String> = music_metas.into_iter().collect();
    let mut regions = Vec::with_capacity(masterdata.len());
    for (name, dir) in masterdata {
        let music = music_by_region.get(&name).ok_or_else(|| {
            format!("region {name} has --masterdata but no matching --music-metas")
        })?;
        regions.push(RegionSource {
            name,
            masterdata_dir: PathBuf::from(dir),
            music_metas: PathBuf::from(music),
        });
    }

    let first_region = regions
        .first()
        .map(|region| region.name.clone())
        .unwrap_or_default();
    let default_region = value(&single, "default-region").unwrap_or(first_region);
    if !regions.iter().any(|region| region.name == default_region) {
        return Err(format!(
            "--default-region {default_region} has no masterdata"
        ));
    }

    let workers = match value(&single, "workers") {
        Some(text) => parse_usize("--workers", &text)?,
        None => std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(1),
    };
    if workers == 0 {
        return Err("--workers must be at least 1".to_string());
    }

    let bind_text = value(&single, "bind").unwrap_or_else(|| DEFAULT_BIND.to_string());
    let bind: SocketAddr = bind_text
        .parse()
        .map_err(|error| format!("--bind {bind_text} is not an address: {error}"))?;

    let log_format = value(&single, "log-format").unwrap_or_else(|| "text".to_string());
    let log_json = match log_format.as_str() {
        "text" => false,
        "json" => true,
        other => return Err(format!("--log-format must be text or json, got {other}")),
    };

    let max_queue = opt_usize(&single, "max-queue")?
        .unwrap_or(workers.saturating_mul(8))
        .max(1);
    let max_limit = opt_usize(&single, "max-limit")?
        .unwrap_or(DEFAULT_MAX_LIMIT)
        .max(1);

    Ok(Parsed::Run(Box::new(Config {
        bind,
        regions,
        default_region,
        workers,
        max_queue,
        queue_timeout_ms: opt_u64(&single, "queue-timeout-ms")?.unwrap_or(DEFAULT_QUEUE_TIMEOUT_MS),
        max_search_timeout_ms: opt_u64(&single, "max-search-timeout-ms")?
            .unwrap_or(DEFAULT_MAX_SEARCH_TIMEOUT_MS),
        max_limit,
        max_body_bytes: opt_usize(&single, "max-body-bytes")?.unwrap_or(DEFAULT_MAX_BODY_BYTES),
        admin_token: value(&single, "admin-token"),
        log_json,
    })))
}

fn take(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn split_pair(text: &str) -> Result<(String, String), String> {
    match text.split_once('=') {
        Some((name, value)) if !name.is_empty() && !value.is_empty() => {
            Ok((name.to_string(), value.to_string()))
        }
        _ => Err(format!("expected <region>=<path>, got {text}")),
    }
}

/// Reads a repeatable option from the environment as a comma-separated list.
fn env_pairs(variable: &str) -> Result<Vec<(String, String)>, String> {
    let Ok(text) = std::env::var(variable) else {
        return Ok(Vec::new());
    };
    text.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(split_pair)
        .collect()
}

/// Flag first, then `ALLIUM_DECK_<KEY>`.
fn value(single: &BTreeMap<String, String>, key: &str) -> Option<String> {
    if let Some(found) = single.get(key) {
        return Some(found.clone());
    }
    let variable = format!("ALLIUM_DECK_{}", key.to_uppercase().replace('-', "_"));
    std::env::var(variable).ok().filter(|text| !text.is_empty())
}

fn opt_usize(single: &BTreeMap<String, String>, key: &str) -> Result<Option<usize>, String> {
    match value(single, key) {
        None => Ok(None),
        Some(text) => parse_usize(key, &text).map(Some),
    }
}

fn opt_u64(single: &BTreeMap<String, String>, key: &str) -> Result<Option<u64>, String> {
    match value(single, key) {
        None => Ok(None),
        Some(text) => text
            .parse::<u64>()
            .map(Some)
            .map_err(|error| format!("{key} {text} is not a number: {error}")),
    }
}

fn parse_usize(key: &str, text: &str) -> Result<usize, String> {
    text.parse::<usize>()
        .map_err(|error| format!("{key} {text} is not a number: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|text| (*text).to_string()).collect()
    }

    #[test]
    fn masterdata_and_music_metas_pair_up_by_region() {
        let parsed = parse(&args(&[
            "--masterdata",
            "cn=/data/cn",
            "--music-metas",
            "cn=/data/cn.json",
            "--workers",
            "2",
        ]));
        let Ok(Parsed::Run(config)) = parsed else {
            panic!("expected a runnable config");
        };
        assert_eq!(config.regions.len(), 1);
        assert_eq!(config.regions[0].name, "cn");
        assert_eq!(config.default_region, "cn");
        assert_eq!(config.workers, 2);
        // max-queue defaults to workers * 8.
        assert_eq!(config.max_queue, 16);
    }

    #[test]
    fn masterdata_without_music_metas_is_rejected() {
        let error = parse(&args(&["--masterdata", "cn=/data/cn"]));
        assert!(matches!(error, Err(message) if message.contains("no matching")));
    }

    #[test]
    fn default_region_must_have_masterdata() {
        let error = parse(&args(&[
            "--masterdata",
            "cn=/data/cn",
            "--music-metas",
            "cn=/data/cn.json",
            "--default-region",
            "jp",
        ]));
        assert!(matches!(error, Err(message) if message.contains("--default-region jp")));
    }

    #[test]
    fn region_pairs_need_both_halves() {
        assert!(split_pair("cn=").is_err());
        assert!(split_pair("=/data/cn").is_err());
        assert!(split_pair("cn").is_err());
        assert_eq!(
            split_pair("cn=/data/cn"),
            Ok(("cn".to_string(), "/data/cn".to_string()))
        );
    }

    #[test]
    fn help_and_version_short_circuit() {
        assert!(matches!(parse(&args(&["--help"])), Ok(Parsed::Help(_))));
        assert!(matches!(parse(&args(&["-V"])), Ok(Parsed::Version(_))));
    }
}
