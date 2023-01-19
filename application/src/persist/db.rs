use std::{sync::Mutex, path::Path};

use super::*;
use serde::{de::DeserializeOwned, Serialize};

pub struct DB {
    connection: Mutex<rusqlite::Connection>,
}

impl DB {
    pub fn open<P: AsRef<Path>>(location: P) -> Result<Self, PersistError> {
        let connection = Mutex::new(rusqlite::Connection::open(location)?);
        Ok(Self { connection })
    }

    fn table_for<T: Serialize>() -> &'static str {
        std::any::type_name::<T>().rsplit_once(':').unwrap().1
    }

    fn schema_for(schema: Option<&str>) -> &str {
        schema.unwrap_or("main")
    }

    pub fn create_table<T: Serialize + Default>(&self) -> Result<(), PersistError> {
        self.create_table_schema::<T>(None)
    }

    pub fn create_table_schema<T: Serialize + Default>(&self, schema: Option<&str>) -> Result<(), PersistError> {
        use rusqlite::types::ToSqlOutput::*;
        use rusqlite::types::*;
        let t = T::default();
        let params = serde_rusqlite::to_params_named(&t)?;
        let mut types = vec![];
        for (name, value) in params.to_slice() {
            let value = value.to_sql()?;
            types.push(format!(
                "{} {} not null",
                name.trim_start_matches(':'),
                match value.to_sql()? {
                    Owned(Value::Integer(..)) | Borrowed(ValueRef::Integer(..)) => {
                        "int"
                    }
                    Owned(Value::Text(..)) | Borrowed(ValueRef::Text(..)) => {
                        "text"
                    }
                    Owned(Value::Real(..)) | Borrowed(ValueRef::Real(..)) => {
                        "real"
                    }
                    Owned(Value::Blob(..)) | Borrowed(ValueRef::Blob(..)) => {
                        "blob"
                    }
                    _ => {
                        return Err(PersistError::Unmappable());
                    }
                }
            ));
        }
        let sql = format!(
            "create table if not exists {}.{}({})",
            Self::schema_for(schema),
            Self::table_for::<T>(),
            types.join(",")
        );
        self.connection.lock().expect("Poisoned").execute(&sql, ())?;
        Ok(())
    }


    pub fn create_unique_index<T: Serialize + Default>(&self, name: &str, keys: &[&str]) -> Result<(), PersistError> {
        self.create_unique_index_schema::<T>(None, name, keys)
    }

    pub fn create_unique_index_schema<T: Serialize + Default>(&self, schema: Option<&str>, name: &str, keys: &[&str]) -> Result<(), PersistError> {
        let t = T::default();
        let params = serde_rusqlite::to_params_named(&t)?;
        // TODO: check keys
        let sql = format!("create unique index if not exists {} on {}({})", name, Self::table_for::<T>(), keys.join(","));
        self.connection.lock().expect("Poisoned").execute(&sql, ())?;
        Ok(())
    }

    pub fn store<T: Serialize>(&self, t: &T) -> Result<(), PersistError> {
        self.store_schema(None, t)
    }

    pub fn store_schema<T: Serialize>(&self, schema: Option<&str>, t: &T) -> Result<(), PersistError> {
        let params = serde_rusqlite::to_params_named(t)?;
        let params_slice = params.to_slice();
        let columns = params_slice
            .iter()
            .map(|f| f.0.trim_start_matches(':'))
            .collect::<Vec<_>>()
            .join(",");
        let values = params_slice
            .iter()
            .map(|f| f.0)
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "insert or replace into {}.{}({}) values ({})",
            Self::schema_for(schema),
            Self::table_for::<T>(),
            columns,
            values
        );
        self.connection.lock().expect("Poisoned").execute(&sql, params_slice.as_slice())?;
        Ok(())
    }

    pub fn load<T: Serialize + DeserializeOwned>(
        &self,
        id: String,
    ) -> Result<Option<T>, PersistError> {
        self.load_schema(None, id)
    }

    pub fn load_schema<T: Serialize + DeserializeOwned>(
        &self,
        schema: Option<&str>,
        id: String,
    ) -> Result<Option<T>, PersistError> {
        let sql = format!("select * from {}.{} where id = ?", Self::schema_for(schema), Self::table_for::<T>());
        match self
            .connection
            .lock().expect("Poisoned")
            .query_row_and_then(&sql, [id], |row| serde_rusqlite::from_row::<T>(row))
        {
            Err(serde_rusqlite::Error::Rusqlite(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
            Err(x) => Err(PersistError::SerdeError(x)),
            Ok(x) => Ok(Some(x)),
        }
    }

    pub fn execute_raw(&mut self, sql: &str) -> Result<(), PersistError> {
        self.connection.lock().expect("Poisoned").execute_batch(sql)?;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Deserialize, Serialize, Debug, Default, Eq, PartialEq)]
    struct TestSerialize {
        id: String,
        integer: u32,
        string: String,
    }

    #[test]
    fn load_store() {
        let mut db = DB::open(":memory:").unwrap();
        db.create_table::<TestSerialize>().unwrap();
        let input = TestSerialize {
            id: "a".into(),
            integer: 123,
            string: "hello".into(),
        };
        db.store(&input).unwrap();
        let output = db.load("a".into()).unwrap();
        assert_eq!(Some(input), output);
    }

    #[test]
    fn load_store_missing() {
        let mut db = DB::open(":memory:").unwrap();
        db.create_table::<TestSerialize>().unwrap();
        let output = db.load::<TestSerialize>("a".into()).unwrap();
        assert_eq!(None, output);
    }
}
