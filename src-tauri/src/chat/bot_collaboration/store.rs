//! Scoped collaboration records. The service mutex is the only writer.
//!
//! Each delivery, room, exchange, and pending action is its own file. A
//! mutation writes a transaction for the records it touched, then applies
//! that transaction. Unchanged history is not cloned or rewritten.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::types::{Delivery, DiskState, Exchange, PendingAction, Room};

pub struct Store {
    dir: PathBuf,
}

#[derive(Debug)]
pub enum CommitError {
    /// `tx.json` was not replaced. The previous files are still the state.
    NotDurable(String),
    /// `tx.json` is durable. Reloading applies it even if a record file is behind.
    Durable(String),
}

pub struct Loaded {
    pub state: DiskState,
    pub legacy: bool,
}

#[derive(Debug, Clone, Default)]
pub struct RecordBatch {
    pub put_deliveries: Vec<Delivery>,
    pub put_rooms: Vec<Room>,
    pub put_exchanges: Vec<Exchange>,
    pub put_pending: Vec<PendingAction>,
    pub delete_deliveries: Vec<String>,
    pub delete_rooms: Vec<String>,
    pub delete_exchanges: Vec<String>,
    pub delete_pending: Vec<String>,
    pub replaces_legacy: bool,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Tx {
    #[serde(default)]
    put_deliveries: Vec<Delivery>,
    #[serde(default)]
    put_rooms: Vec<Room>,
    #[serde(default)]
    put_exchanges: Vec<Exchange>,
    #[serde(default)]
    put_pending: Vec<PendingAction>,
    #[serde(default)]
    delete_deliveries: Vec<String>,
    #[serde(default)]
    delete_rooms: Vec<String>,
    #[serde(default)]
    delete_exchanges: Vec<String>,
    #[serde(default)]
    delete_pending: Vec<String>,
    #[serde(default)]
    replaces_legacy: bool,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn load(&self) -> Result<Option<Loaded>, String> {
        let tx = self.read_tx()?;
        let records_exist = self.records_present();
        let legacy_state = if records_exist {
            None
        } else {
            self.read_legacy_if_present()?
        };
        let mut state = if records_exist {
            self.read_records()?
        } else {
            legacy_state.clone().unwrap_or_default()
        };
        let legacy = legacy_state.is_some();
        if let Some(tx) = tx {
            apply_tx(&mut state, &tx);
            if let Err(err) = self.materialize(&tx) {
                eprintln!("[bot-collaboration] 事务还没有落到每条记录：{err}");
            }
        }
        if state.deliveries.is_empty()
            && state.rooms.is_empty()
            && state.exchanges.is_empty()
            && state.pending.is_empty()
            && !legacy
        {
            return Ok(None);
        }
        Ok(Some(Loaded { state, legacy }))
    }

    pub fn commit(&self, batch: &RecordBatch) -> Result<(), CommitError> {
        if self.tx_path().is_file() {
            self.finish_existing_tx()?;
        }
        let tx = Tx::from(batch);
        let text = serde_json::to_string_pretty(&tx)
            .map_err(|err| CommitError::NotDurable(err.to_string()))?;
        crate::chat::storage::atomic_write(&self.tx_path(), &text, "bot collaboration transaction")
            .map_err(CommitError::NotDurable)?;
        if let Err(err) = self.materialize(&tx) {
            return Err(CommitError::Durable(err));
        }
        match fs::remove_file(self.tx_path()) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(CommitError::Durable(format!(
                "协作事务已写入，但事务文件还在：{err}"
            ))),
        }
    }

    fn finish_existing_tx(&self) -> Result<(), CommitError> {
        let Some(tx) = self.read_tx().map_err(CommitError::NotDurable)? else {
            return Ok(());
        };
        self.materialize(&tx).map_err(CommitError::Durable)?;
        match fs::remove_file(self.tx_path()) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(CommitError::Durable(err.to_string())),
        }
    }

    fn materialize(&self, tx: &Tx) -> Result<(), String> {
        for id in &tx.delete_deliveries {
            remove_record(&self.record_path("deliveries", id)?)?;
        }
        for id in &tx.delete_rooms {
            remove_record(&self.record_path("rooms", id)?)?;
        }
        for id in &tx.delete_exchanges {
            remove_record(&self.record_path("exchanges", id)?)?;
        }
        for id in &tx.delete_pending {
            remove_record(&self.record_path("pending", id)?)?;
        }
        for delivery in &tx.put_deliveries {
            write_record(&self.record_path("deliveries", &delivery.id)?, delivery)?;
        }
        for room in &tx.put_rooms {
            write_record(&self.record_path("rooms", &room.id)?, room)?;
        }
        for exchange in &tx.put_exchanges {
            write_record(&self.record_path("exchanges", &exchange.id)?, exchange)?;
        }
        for action in &tx.put_pending {
            write_record(&self.record_path("pending", &action.id)?, action)?;
        }
        if tx.replaces_legacy && self.legacy_path().is_file() {
            match fs::remove_file(self.legacy_path()) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(format!("新的协作记录已写入，但旧文件还在：{err}"));
                }
            }
        }
        Ok(())
    }

    fn read_records(&self) -> Result<DiskState, String> {
        Ok(DiskState {
            deliveries: read_kind(&self.dir.join("collaboration").join("deliveries"))?,
            rooms: read_kind(&self.dir.join("collaboration").join("rooms"))?,
            exchanges: read_kind(&self.dir.join("collaboration").join("exchanges"))?,
            pending: read_kind(&self.dir.join("collaboration").join("pending"))?,
        })
    }

    fn read_legacy_if_present(&self) -> Result<Option<DiskState>, String> {
        let path = self.legacy_path();
        match fs::metadata(&path) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(format!("无法读取 Bot 协作数据 {}：{err}", path.display()));
            }
            Ok(meta) if !meta.is_file() => {
                return Err(format!(
                    "无法读取 Bot 协作数据 {}：不是协作文件",
                    path.display()
                ));
            }
            Ok(_) => {}
        }
        self.read_legacy().map(Some)
    }

    fn read_legacy(&self) -> Result<DiskState, String> {
        let path = self.legacy_path();
        let text = fs::read_to_string(&path)
            .map_err(|err| format!("无法读取 Bot 协作数据 {}：{err}", path.display()))?;
        serde_json::from_str(&text)
            .map_err(|err| format!("无法读取 Bot 协作数据 {}：{err}", path.display()))
    }

    fn read_tx(&self) -> Result<Option<Tx>, String> {
        let path = self.tx_path();
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(format!("无法读取 Bot 协作事务 {}：{err}", path.display()));
            }
        };
        serde_json::from_str(&text)
            .map(Some)
            .map_err(|err| format!("无法读取 Bot 协作事务 {}：{err}", path.display()))
    }

    fn records_present(&self) -> bool {
        ["deliveries", "rooms", "exchanges", "pending"]
            .into_iter()
            .any(|kind| self.dir.join("collaboration").join(kind).is_dir())
    }

    fn legacy_path(&self) -> PathBuf {
        self.dir.join("collaboration.json")
    }

    fn tx_path(&self) -> PathBuf {
        self.dir.join("collaboration").join("tx.json")
    }

    fn record_path(&self, kind: &str, id: &str) -> Result<PathBuf, String> {
        Ok(self
            .dir
            .join("collaboration")
            .join(kind)
            .join(format!("{}.json", file_id(id)?)))
    }
}

/// Test-only locator for one delivery file. Production writes stay inside `Store`.
#[cfg(test)]
pub fn delivery_record_path(dir: &Path, id: &str) -> PathBuf {
    dir.join("collaboration")
        .join("deliveries")
        .join(format!("{id}.json"))
}

impl From<&RecordBatch> for Tx {
    fn from(batch: &RecordBatch) -> Self {
        Self {
            put_deliveries: batch.put_deliveries.clone(),
            put_rooms: batch.put_rooms.clone(),
            put_exchanges: batch.put_exchanges.clone(),
            put_pending: batch.put_pending.clone(),
            delete_deliveries: batch.delete_deliveries.clone(),
            delete_rooms: batch.delete_rooms.clone(),
            delete_exchanges: batch.delete_exchanges.clone(),
            delete_pending: batch.delete_pending.clone(),
            replaces_legacy: batch.replaces_legacy,
        }
    }
}

fn apply_tx(state: &mut DiskState, tx: &Tx) {
    for id in &tx.delete_deliveries {
        state.deliveries.retain(|delivery| &delivery.id != id);
    }
    for id in &tx.delete_rooms {
        state.rooms.retain(|room| &room.id != id);
    }
    for id in &tx.delete_exchanges {
        state.exchanges.retain(|exchange| &exchange.id != id);
    }
    for id in &tx.delete_pending {
        state.pending.retain(|action| &action.id != id);
    }
    upsert(&mut state.deliveries, &tx.put_deliveries, |item| {
        item.id.clone()
    });
    upsert(&mut state.rooms, &tx.put_rooms, |item| item.id.clone());
    upsert(&mut state.exchanges, &tx.put_exchanges, |item| {
        item.id.clone()
    });
    upsert(&mut state.pending, &tx.put_pending, |item| item.id.clone());
}

fn upsert<T: Clone>(rows: &mut Vec<T>, puts: &[T], id_of: impl Fn(&T) -> String) {
    for item in puts {
        let id = id_of(item);
        if let Some(existing) = rows.iter_mut().find(|row| id_of(row) == id) {
            *existing = item.clone();
        } else {
            rows.push(item.clone());
        }
    }
}

fn write_record<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|err| err.to_string())?;
    crate::chat::storage::atomic_write(path, &text, "bot collaboration record")
}

fn remove_record(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("无法删除协作记录 {}：{err}", path.display())),
    }
}

fn read_kind<T: for<'de> Deserialize<'de>>(dir: &Path) -> Result<Vec<T>, String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => {
            return Err(format!("无法读取 Bot 协作数据 {}：{err}", dir.display()));
        }
    };
    let mut rows = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|err| format!("无法读取 Bot 协作数据 {}：{err}", dir.display()))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let text = fs::read_to_string(&path)
            .map_err(|err| format!("无法读取 Bot 协作数据 {}：{err}", path.display()))?;
        let row = serde_json::from_str(&text)
            .map_err(|err| format!("无法读取 Bot 协作数据 {}：{err}", path.display()))?;
        rows.push(row);
    }
    Ok(rows)
}

fn file_id(id: &str) -> Result<&str, String> {
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id.contains("..")
        || id.contains('\0')
    {
        return Err(format!("协作记录 id 不能作为文件名：{id}"));
    }
    Ok(id)
}
