use crate::infrastructure::postgres::Error as DbError;
use serde::Deserialize;
use std::{collections::HashSet, hash::Hash};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection<T> {
    pub ids: Option<Vec<T>>,
    pub query: Option<String>,
}
impl<T: Eq + Hash> Selection<T> {
    pub fn validate(&self, expected: i64) -> Result<(), Error> {
        if expected < 1
            || match (&self.ids, &self.query) {
                (Some(ids), None) => {
                    ids.is_empty()
                        || ids.len() > 65536
                        || ids.iter().collect::<HashSet<_>>().len() != ids.len()
                        || expected != ids.len() as i64
                }
                (None, Some(query)) => query.len() > 1024,
                _ => true,
            }
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn patterns(&self) -> Vec<String> {
        super::patterns::sql(self.query.as_deref().unwrap_or(""))
    }
}
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("choose either distinct IDs or a search query and its exact match count")]
    Invalid,
    #[error("selection changed or includes unavailable items; reload and select again")]
    Changed,
    #[error("switch this workspace to selected tools before changing tool inclusion")]
    Scope,
    #[error(transparent)]
    Database(#[from] DbError),
}
