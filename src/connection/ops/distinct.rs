//! Distinct field-value queries for MongoDB collections.

use mongodb::Client;
use mongodb::bson::{Bson, Document};

use crate::connection::ConnectionManager;
use crate::error::Result;

impl ConnectionManager {
    /// Return distinct values for `field` in a collection, optionally filtered.
    pub fn distinct_values(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        field: &str,
        filter: Option<Document>,
    ) -> Result<Vec<Bson>> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let field = field.to_string();

        self.runtime.block_on(async move {
            let coll = client.database(&database).collection::<Document>(&collection);
            let values = match filter {
                Some(filter) => coll.distinct(&field, filter).await?,
                None => coll.distinct(&field, Document::new()).await?,
            };
            Ok(values)
        })
    }
}
