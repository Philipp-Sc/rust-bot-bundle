use super::sled_store::SledStore;

use log::{debug, trace};
use sled::{IVec, Subscriber};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use std::{
    error::Error as StdError,
    fmt::{self, Display},
};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Entry<T> {
    pub data: Result<T, EntryError>,
    pub timestamp: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct IgnoreEntry {}

impl TryFrom<Vec<u8>> for IgnoreEntry {
    type Error = ();
    fn try_from(_item: Vec<u8>) -> Result<Self, Self::Error> {
        Ok(IgnoreEntry {})
    }
}

impl<T: std::cmp::PartialEq> Entry<T> {
    fn new(data: Result<T, EntryError>) -> Self {
        Self {
            data,
            timestamp: Utc::now().timestamp(),
        }
    }
    fn is_same_data(&self, data: &Result<T, EntryError>) -> bool {
        return &self.data == data;
    }
}

impl<T: for<'a> Deserialize<'a>> TryFrom<Vec<u8>> for Entry<T> {
    type Error = anyhow::Error;
    fn try_from(item: Vec<u8>) -> anyhow::Result<Self> {
        Ok(bincode::deserialize(&item[..])?)
    }
}

impl<T: Serialize> TryFrom<Entry<T>> for Vec<u8> {
    type Error = anyhow::Error;
    fn try_from(item: Entry<T>) -> anyhow::Result<Self> {
        Ok(bincode::serialize(&item)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EntryError {
    NotYetResolved(String),
    KeyDoesNotExist(String),
    EntryReserved(String),
    Error(String),
}

impl StdError for EntryError {}

impl Display for EntryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::result::Result<(), fmt::Error> {
        use self::EntryError::*;

        match *self {
            NotYetResolved(ref key) => write!(f, "Error: Not yet resolved!: {}", key),
            KeyDoesNotExist(ref key) => write!(f, "Error: Key does not exist!: {}", key),
            EntryReserved(ref key) => write!(f, "Error: Entry reserved!: {}", key),
            Error(ref text) => write!(f, "{}", text),
        }
    }
}

#[derive(Debug)]
pub enum RetrievalMethod {
    Get,
    GetOk,
}

pub struct FallbackEntryStore(SledStore);

const REV_INDEX_PREFIX: &str = "rev_index_";
const KEY_PREFIX: &str = "key_";

impl Clone for FallbackEntryStore {
    fn clone(&self) -> Self {
        let sled_db_copy = self.0.clone();
        FallbackEntryStore(sled_db_copy)
    }
}

impl FallbackEntryStore {
    pub fn new(sled_db: &sled::Db, global_prefix: &str) -> Self {
        FallbackEntryStore(SledStore::new(sled_db, global_prefix))
    }

    // Get: returns the max revision.
    // GetOk: returns the first ok result with max revision
    // the item stored and found with the given key must impl Deserialize for T, else an Error is returned.
    pub fn get<T>(&self, key: &str, retrieval_method: &RetrievalMethod) -> anyhow::Result<Entry<T>>
    where
        T: for<'a> Deserialize<'a> + Serialize,
    {
        let current_rev: Option<IVec> = self
            .0
            .get(format!("{}{}", REV_INDEX_PREFIX, key).as_bytes().to_vec())?;
        let index = match current_rev {
            Some(val) => u64::from_be_bytes(val.to_vec()[..].try_into()?),
            None => 0u64,
        };

        let value = match retrieval_method {
            RetrievalMethod::Get => {
                let key = format!("{}{}_rev_{}", KEY_PREFIX, key, index);
                trace!("Get: {},", key);
                let item: Option<IVec> = self.0.get(key.as_bytes().to_vec())?;
                Ok(match item {
                    Some(val) => val.to_vec().try_into()?,
                    None => Entry {
                        data: Err(EntryError::KeyDoesNotExist(key.to_string())),
                        timestamp: Utc::now().timestamp(),
                    },
                })
            }
            RetrievalMethod::GetOk => {
                for i in (0..=index).rev() {
                    let key = format!("{}{}_rev_{}", KEY_PREFIX, key, i);
                    trace!("GetOk: {}", key);
                    let item: Option<IVec> = self.0.get(key.as_bytes().to_vec())?;
                    match item {
                        Some(val) => {
                            let tmp: Entry<T> = val.to_vec().try_into()?;
                            if let Entry { data: Ok(_), .. } = tmp {
                                return Ok(tmp);
                            }
                        }
                        None => {
                            break;
                        }
                    }
                }
                Err(anyhow::anyhow!("Error: no ok value found for key {}", key))
            }
        };
        trace!(
            "{:?}: key: {:?}, value: {}",
            retrieval_method,
            key,
            match &value {
                Ok(v) => serde_json::to_string_pretty(v).unwrap_or("Formatting Error".to_string()),
                Err(e) => e.to_string(),
            }
        );
        value
    }

    pub fn get_index_of_ok_result<T>(&self, key: &str, index: u64) -> anyhow::Result<u64>
    where
        T: for<'a> Deserialize<'a> + Serialize,
    {
        for i in (0..=index).rev() {
            let key = format!("{}{}_rev_{}", KEY_PREFIX, key, i);
            let item: Option<IVec> = self.0.get(key.as_bytes().to_vec())?;
            match item {
                Some(val) => {
                    let tmp: Entry<T> = val.to_vec().try_into()?;
                    if let Entry { data: Ok(_), .. } = tmp {
                        return Ok(i);
                    }
                }
                None => {}
            }
        }
        Err(anyhow::anyhow!("Error: no index found for key {}", key))
    }

    pub fn contains_key(&self, key: &str) -> bool {
        let current_rev: Option<Option<IVec>> = self
            .0
            .get(format!("{}{}", REV_INDEX_PREFIX, key).as_bytes().to_vec())
            .ok();
        let res = match current_rev {
            Some(Some(_val)) => true,
            Some(None) => false,
            None => false,
        };
        trace!("contains_key: key: {}, value: {:?}", key, res);
        res
    }

    // Removes entries, starting from the last successful result (exclusive).
    // It removes entries with revision indices lower than the provided max_index.
    // This method is typically used to clean up the revision history and remove older entries that are no longer needed, improving performance and reducing storage space.
    pub fn cleanup_revision_history<T>(&self, key: &str, max_index: u64) -> anyhow::Result<()>
    where
        T: for<'a> Deserialize<'a> + Serialize,
    {
        trace!(
            "remove_historic_entries: key: {}, max_index: {}",
            key,
            max_index
        );
        let smallest_required_index = self
            .get_index_of_ok_result::<T>(key, max_index)
            .unwrap_or(max_index);
        for i in (0..smallest_required_index).rev() {
            if self
                .0
                .remove(
                    format!("{}{}_rev_{}", KEY_PREFIX, &key, i)
                        .as_bytes()
                        .to_vec(),
                )?
                .is_none()
            {
                trace!("key does not exist: key: {}, index: {}", key, i);
                break;
            } else {
                trace!("removed: key: {}, index: {}", key, i);
            }
        }
        Ok(())
    }

    // Function to insert if data doesn't exist for the given key
    pub fn insert_if_not_exists<T>(
        &self,
        key: &str,
        data: Result<T, EntryError>,
    ) -> anyhow::Result<bool>
    where
        T: for<'a> Deserialize<'a> + Serialize + std::cmp::PartialEq,
    {
        // Check if data already exists for the given key
        let existing_data: anyhow::Result<Entry<T>> = self.get::<T>(key, &RetrievalMethod::GetOk);

        let mut insert = false;
        if let Ok(my_data) = existing_data {
            if !my_data.is_same_data(&data) {
                insert = true;
            }
        } else {
            insert = true;
        }
        if insert {
            self.insert(key, data)?;
        }
        Ok(insert)
    }

    // increases revision and adds key/value pair to it.
    // uses `remove_historic_entries` to clean up the history.
    //
    // called in async/parallel from multiple threads
    pub fn insert<T>(&self, key: &str, data: Result<T, EntryError>) -> anyhow::Result<()>
    where
        T: for<'a> Deserialize<'a> + Serialize + std::cmp::PartialEq,
    {
        let value = Entry::new(data);
        debug!("push: key: {}", key);

        let json_str = serde_json::to_string(&value).unwrap_or("Formatting Error".to_string());
        let truncated_json_str = json_str.chars().take(100).collect::<String>();
        debug!("push: val: {}", truncated_json_str);

        trace!(
            "push: value: {}",
            serde_json::to_string_pretty(&value).unwrap_or("Formatting Error".to_string())
        );
        let current_rev: Option<IVec> = self
            .0
            .get(format!("{}{}", REV_INDEX_PREFIX, key).as_bytes().to_vec())?;
        let next_index = match current_rev {
            Some(val) => u64::from_be_bytes(val.to_vec()[..].try_into()?).overflowing_add(1),
            None => (0u64, false),
        };
        if next_index.1 {
            // in case of an overflow, the complete key history is wiped.
            trace!("push key: {}, overflow: {:?}", key, next_index);
            for i in (0..=u64::MAX).rev() {
                if self
                    .0
                    .remove(
                        format!("{}{}_rev_{}", KEY_PREFIX, key, i)
                            .as_bytes()
                            .to_vec(),
                    )?
                    .is_none()
                {
                    break;
                } else {
                    trace!("removed: key: {}, index: {}", key, i);
                }
            }
        }
        let tmp: Vec<u8> = value.try_into()?;
        self.0.insert(
            format!("{}{}_rev_{}", KEY_PREFIX, key, next_index.0)
                .as_bytes()
                .to_vec(),
            tmp,
        )?;
        self.0.insert(
            format!("{}{}", REV_INDEX_PREFIX, key).as_bytes().to_vec(),
            next_index.0.to_be_bytes().to_vec(),
        )?;

        self.cleanup_revision_history::<T>(key, next_index.0)?;
        Ok(())
    }

    pub fn remove_all(&self, key: &str) -> anyhow::Result<u64> {
        let current_rev: Option<IVec> = self
            .0
            .get(format!("{}{}", REV_INDEX_PREFIX, key).as_bytes().to_vec())?;
        let max_index = match current_rev {
            Some(val) => u64::from_be_bytes(val.to_vec()[..].try_into()?),
            None => return Ok(0),
        };

        for i in 0..=max_index {
            self.0.remove(
                format!("{}{}_rev_{}", KEY_PREFIX, key, i)
                    .as_bytes()
                    .to_vec(),
            )?;
        }
        self.0
            .remove(format!("{}{}", REV_INDEX_PREFIX, key).as_bytes().to_vec())?;

        Ok(max_index + 1)
    }

    pub fn watch_prefix(&self, prefix: &mut Vec<u8>) -> Subscriber {
        *prefix = vec![KEY_PREFIX.as_bytes(), prefix].concat();
        self.0.watch_prefix(prefix)
    }

    pub fn key_iter<'a>(
        &'a self,
        key_prefix: Option<&'a str>,
    ) -> impl Iterator<Item = String> + '_ {
        let prefix = format!("{}{}", REV_INDEX_PREFIX, key_prefix.unwrap_or(""));
        let iter = self.0.scan_prefix(REV_INDEX_PREFIX.as_bytes());
        iter.filter_map(move |x| {
            if let Ok((key, _)) = x {
                if key.starts_with(&prefix) {
                    return Some(key[REV_INDEX_PREFIX.len()..].to_string());
                }
            }
            return None;
        })
    }

    pub fn value_iter<'b, T>(
        &'b self,
        key_prefix: Option<&'b str>,
        retrieval_method: &'b RetrievalMethod,
    ) -> impl Iterator<Item = (String, Entry<T>)> + '_
    where
        T: for<'a> Deserialize<'a> + Serialize,
    {
        self.key_iter(key_prefix)
            .map(|key| match self.get::<T>(&key, retrieval_method) {
                Ok(val) => (key, val),
                Err(err) => {
                    let error = Err(EntryError::Error(format!(
                        "Error: Key: {}, Err: {}",
                        &key,
                        err.to_string()
                    )));
                    (
                        key,
                        Entry {
                            data: error,
                            timestamp: Utc::now().timestamp(),
                        },
                    )
                }
            })
    }

    pub fn error_iter<'b>(
        &'b self,
        key_prefix: Option<&'b str>,
    ) -> impl Iterator<Item = (String, EntryError)> + '_ {
        self.key_iter(key_prefix).filter_map(|key| {
            match self.get::<IgnoreEntry>(&key, &RetrievalMethod::Get) {
                Ok(Entry {
                    data: Err(error), ..
                }) => Some((key, error)),
                Err(err) => Some((key, EntryError::Error(err.to_string()))),
                _ => None,
            }
        })
    }
}
