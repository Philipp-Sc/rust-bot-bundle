use log::{error, trace};
use sled::{IVec, Subscriber};

pub struct SledStore {
    db: sled::Db,
    global_prefix: String,
}

impl Clone for SledStore {
    fn clone(&self) -> Self {
        SledStore::new(&self.db, self.global_prefix.as_str())
    }
}

impl SledStore {
    pub fn new(sled_db: &sled::Db, global_prefix: &str) -> Self {
        SledStore {
            db: sled_db.clone(),
            global_prefix: global_prefix.to_string(),
        }
    }

    // Helper function to add the global prefix to a key
    fn add_global_prefix<K>(&self, key: K) -> anyhow::Result<String>
    where
        K: AsRef<Vec<u8>>,
    {
        let key_str = std::str::from_utf8(&key.as_ref()[..]).map_err(|err| {
            error!("Error converting key to UTF-8: {}", err);
            anyhow::anyhow!(err.to_string())
        })?;
        Ok(format!("{}{}", self.global_prefix, key_str))
    }

    pub fn contains_key<K>(&self, key: K) -> anyhow::Result<bool>
    where
        K: AsRef<Vec<u8>>,
    {
        let global_key = match self.add_global_prefix(key) {
            Ok(key) => key,
            Err(err) => {
                error!("Error adding global prefix to key: {}", err);
                return Err(err);
            }
        };

        trace!("contains_key {:?}", global_key);
        Ok(self.db.contains_key(global_key.as_bytes())?)
    }

    pub fn get<K>(&self, key: K) -> anyhow::Result<Option<IVec>>
    where
        K: AsRef<Vec<u8>>,
    {
        let global_key = match self.add_global_prefix(key) {
            Ok(key) => key,
            Err(err) => {
                error!("Error adding global prefix to key: {}", err);
                return Err(err);
            }
        };

        trace!("get {:?}", global_key);
        Ok(self.db.get(global_key.as_bytes())?)
    }

    pub fn insert<K, V>(&self, key: K, value: V) -> anyhow::Result<()>
    where
        K: AsRef<Vec<u8>>,
        IVec: From<V>,
    {
        let global_key = match self.add_global_prefix(key) {
            Ok(key) => key,
            Err(err) => {
                error!("Error adding global prefix to key: {}", err);
                return Err(err);
            }
        };

        trace!("inserting {:?}", global_key);
        let _ = self.db.insert(global_key.as_bytes(), value)?;
        Ok(())
    }

    pub fn remove<S>(&self, key: S) -> anyhow::Result<Option<sled::IVec>>
    where
        S: AsRef<Vec<u8>>,
    {
        let global_key = match self.add_global_prefix(key) {
            Ok(key) => key,
            Err(err) => {
                error!("Error adding global prefix to key: {}", err);
                return Err(err);
            }
        };

        trace!("removing {:?} from db", global_key);
        Ok(self.db.remove(global_key.as_bytes())?)
    }

    // Helper function to remove the global prefix from a key
    fn remove_global_prefix(&self, key: &str) -> Option<String> {
        if key.starts_with(&self.global_prefix) {
            Some(key[self.global_prefix.len()..].to_string())
        } else {
            None
        }
    }

    pub fn watch_prefix(&self, prefix: &mut Vec<u8>) -> Subscriber {
        *prefix = vec![self.global_prefix.as_bytes(), prefix].concat();
        self.db.watch_prefix(&prefix[..])
    }

    pub fn scan_prefix<'a>(
        &'a self,
        prefix: &'a [u8],
    ) -> impl Iterator<Item = Result<(String, IVec), sled::Error>> + 'a {
        let global_prefix_bytes = self.global_prefix.as_bytes();
        let adjusted_prefix = [global_prefix_bytes, prefix].concat();
        self.db
            .scan_prefix(&adjusted_prefix)
            .map(move |item| match item {
                Ok((key, value)) => {
                    let adjusted_key =
                        self.remove_global_prefix(std::str::from_utf8(&key).unwrap());
                    Ok((adjusted_key.unwrap_or_default(), value))
                }
                Err(err) => Err(err),
            })
    }
}
