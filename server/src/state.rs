//! Masterdata snapshots kept in memory for the lifetime of the process.
//!
//! One [`RegionSnapshot`] owns everything a request needs for a region: the flattened
//! [`OwnedGameData`], the reusable [`PreparedGameIndexes`] built from it, and the
//! auxiliary tables. Snapshots are immutable once built, so requests share them
//! through an [`Arc`] and `/admin/reload` swaps in a new one atomically.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

use allium_deck::auxiliary::AuxiliaryData;
use allium_deck::engine::{MasterdataSources, OwnedGameData};
use allium_deck::handler::PreparedGameIndexes;

use crate::config::RegionSource;

/// Row counts of the tables a caller is most likely to check after a reload.
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableCounts {
    pub cards: usize,
    pub events: usize,
    pub skills: usize,
    pub music_metas: usize,
    pub world_blooms: usize,
}

/// One region's immutable masterdata, ready to serve requests.
pub struct RegionSnapshot {
    pub name: String,
    owned: OwnedGameData,
    indexes: PreparedGameIndexes,
    auxiliary: AuxiliaryData,
    pub counts: TableCounts,
    pub loaded_at: SystemTime,
    pub load_ms: f64,
}

impl RegionSnapshot {
    /// Reads every `*.json` in the region's masterdata directory and flattens it.
    ///
    /// The raw table text is dropped once parsed, so the resident cost is the
    /// flattened structures rather than both representations.
    pub fn load(source: &RegionSource) -> Result<Self, String> {
        let started = std::time::Instant::now();
        let tables = read_tables(&source.masterdata_dir)?;
        let music_metas = std::fs::read_to_string(&source.music_metas).map_err(|error| {
            format!(
                "region {}: reading {} failed: {error}",
                source.name,
                source.music_metas.display()
            )
        })?;

        // The auxiliary tables are parsed from the same text before it is consumed by
        // `MasterdataSources`; a region without them still serves deck building, and
        // the auxiliary endpoints report the missing table instead.
        let auxiliary = AuxiliaryData::from_strings(&tables)
            .map_err(|error| format!("region {}: {error}", source.name))?;
        let sources = MasterdataSources::from_strings(tables, music_metas);
        let owned = OwnedGameData::from_sources(&sources)
            .map_err(|error| format!("region {}: {error}", source.name))?;

        let counts = TableCounts {
            cards: owned.cards.len(),
            events: owned.events.len(),
            skills: owned.skills.len(),
            music_metas: owned.music_metas.len(),
            world_blooms: owned.world_blooms.len(),
        };
        let indexes = PreparedGameIndexes::new(&owned.as_ref());

        Ok(Self {
            name: source.name.clone(),
            owned,
            indexes,
            auxiliary,
            counts,
            loaded_at: SystemTime::now(),
            load_ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }

    /// Borrowed masterdata view for this snapshot.
    pub fn game(&self) -> allium_deck::handler::GameData<'_> {
        self.owned.as_ref()
    }

    /// Masterdata indexes, reusable across accounts and parameter sets.
    pub fn indexes(&self) -> &PreparedGameIndexes {
        &self.indexes
    }

    /// Auxiliary tables backing the non-deck-building endpoints.
    pub fn auxiliary(&self) -> &AuxiliaryData {
        &self.auxiliary
    }
}

/// Every loaded region plus the one used when a request omits `region`.
pub struct Registry {
    regions: BTreeMap<String, Arc<RegionSnapshot>>,
    default_region: String,
}

impl Registry {
    pub fn load(sources: &[RegionSource], default_region: &str) -> Result<Self, String> {
        let mut regions = BTreeMap::new();
        for source in sources {
            let snapshot = RegionSnapshot::load(source)?;
            regions.insert(source.name.clone(), Arc::new(snapshot));
        }
        Ok(Self {
            regions,
            default_region: default_region.to_string(),
        })
    }

    /// Looks up a region, falling back to the configured default when `name` is `None`.
    pub fn get(&self, name: Option<&str>) -> Option<Arc<RegionSnapshot>> {
        let key = name.unwrap_or(&self.default_region);
        self.regions.get(key).cloned()
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.regions.keys().map(String::as_str)
    }

    pub fn snapshots(&self) -> impl Iterator<Item = &Arc<RegionSnapshot>> {
        self.regions.values()
    }

    pub fn default_region(&self) -> &str {
        &self.default_region
    }
}

/// A registry that can be replaced wholesale while requests are in flight.
pub struct SharedRegistry {
    inner: RwLock<Arc<Registry>>,
}

impl SharedRegistry {
    pub fn new(registry: Registry) -> Self {
        Self {
            inner: RwLock::new(Arc::new(registry)),
        }
    }

    /// Current registry. Requests hold the `Arc`, so a concurrent reload cannot
    /// pull the masterdata out from under them.
    pub fn current(&self) -> Arc<Registry> {
        match self.inner.read() {
            Ok(guard) => Arc::clone(&guard),
            // A panicking reader cannot leave the registry inconsistent: it is only
            // ever read here and replaced wholesale below.
            Err(poisoned) => Arc::clone(&poisoned.into_inner()),
        }
    }

    /// Swaps in a freshly loaded registry.
    pub fn replace(&self, registry: Registry) {
        let next = Arc::new(registry);
        match self.inner.write() {
            Ok(mut guard) => *guard = next,
            Err(poisoned) => *poisoned.into_inner() = next,
        }
    }
}

/// Reads the flat `*.json` tables of a masterdata directory, keyed by file name.
fn read_tables(directory: &Path) -> Result<BTreeMap<String, String>, String> {
    let entries = std::fs::read_dir(directory)
        .map_err(|error| format!("reading {} failed: {error}", directory.display()))?;
    let mut tables = BTreeMap::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("walking {} failed: {error}", directory.display()))?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("reading {} failed: {error}", path.display()))?;
        tables.insert(name.to_string(), text);
    }
    if tables.is_empty() {
        return Err(format!("{} holds no *.json tables", directory.display()));
    }
    Ok(tables)
}
