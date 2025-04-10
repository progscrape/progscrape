use std::path::Path;

use super::*;
use keepcalm::Shared;
use serde::{Serialize, de::DeserializeOwned};

pub struct DB {
    connection: Shared<rusqlite::Connection>,
}

impl DB {
    pub fn open<P: AsRef<Path>>(location: P) -> Result<Self, PersistError> {
        let db = rusqlite::Connection::open(location)?;
        let connection = Shared::new_unsync(db);
        Ok(Self { connection })
    }

    pub fn table_for<T: Serialize>() -> &'static str {
        std::any::type_name::<T>().rsplit_once(':').unwrap().1
    }

    fn schema_for(schema: Option<&str>) -> &str {
        schema.unwrap_or("main")
    }

    pub fn create_table<T: Serialize + Default>(&self) -> Result<(), PersistError> {
        self.create_table_schema::<T>(None)
    }

    pub fn create_table_schema<T: Serialize + Default>(
        &self,
        schema: Option<&str>,
    ) -> Result<(), PersistError> {
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
                        return Err(PersistError::UnexpectedError(
                            "Could not map this column type".into(),
                        ));
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
        self.connection.read().execute(&sql, ())?;
        Ok(())
    }

    pub fn create_unique_index<T: Serialize + Default>(
        &self,
        name: &str,
        keys: &[&str],
    ) -> Result<(), PersistError> {
        self.create_unique_index_schema::<T>(None, name, keys)
    }

    pub fn create_unique_index_schema<T: Serialize + Default>(
        &self,
        _schema: Option<&str>,
        name: &str,
        keys: &[&str],
    ) -> Result<(), PersistError> {
        let t = T::default();
        let _params = serde_rusqlite::to_params_named(&t)?;
        // TODO: check keys
        let sql = format!(
            "create unique index if not exists {} on {}({})",
            name,
            Self::table_for::<T>(),
            keys.join(",")
        );
        self.connection.read().execute(&sql, ())?;
        Ok(())
    }

    pub fn store_batch<T: Serialize>(&self, t: Vec<T>) -> Result<(), PersistError> {
        self.store_batch_schema(None, t)
    }

    pub fn store_batch_schema<T: Serialize>(
        &self,
        schema: Option<&str>,
        t: Vec<T>,
    ) -> Result<(), PersistError> {
        if let Some(first) = t.first() {
            let params = serde_rusqlite::to_params_named(first)?;
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

            let conn = self.connection.read();
            let mut prep = conn.prepare(&sql)?;
            for t in t {
                let params = serde_rusqlite::to_params_named(t)?;
                prep.execute(params.to_slice().as_slice())?;
            }
            prep.finalize()?;
        }
        Ok(())
    }

    pub fn store<T: Serialize>(&self, t: &T) -> Result<(), PersistError> {
        self.store_schema(None, t)
    }

    pub fn store_schema<T: Serialize>(
        &self,
        schema: Option<&str>,
        t: &T,
    ) -> Result<(), PersistError> {
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
        self.connection
            .read()
            .execute(&sql, params_slice.as_slice())?;
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
        let sql = format!(
            "select * from {}.{} where id = ?",
            Self::schema_for(schema),
            Self::table_for::<T>()
        );
        match self
            .connection
            .read()
            .query_row_and_then(&sql, [id], |row| serde_rusqlite::from_row::<T>(row))
        {
            Err(serde_rusqlite::Error::Rusqlite(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
            Err(x) => Err(PersistError::SerdeError(x)),
            Ok(x) => Ok(Some(x)),
        }
    }

    pub fn execute_raw(&self, sql: &str) -> Result<(), PersistError> {
        self.connection.read().execute_batch(sql)?;
        Ok(())
    }

    pub fn query_raw<T: Serialize + DeserializeOwned>(
        &self,
        sql: &str,
    ) -> Result<Vec<T>, PersistError> {
        let mut v = vec![];
        self.query_raw_callback(sql, |x| {
            v.push(x);
            Ok(())
        })?;
        Ok(v)
    }

    pub fn query_raw_callback<
        T: Serialize + DeserializeOwned,
        F: FnMut(T) -> Result<(), PersistError>,
    >(
        &self,
        sql: &str,
        mut f: F,
    ) -> Result<(), PersistError> {
        let db = self.connection.read();
        let mut stmt = db.prepare(sql)?;
        let mut res = stmt.query([])?;
        loop {
            match res.next() {
                Ok(Some(row)) => {
                    f(serde_rusqlite::from_row::<T>(row)?)?;
                }
                Ok(None) => {
                    break;
                }
                x @ Err(_) => {
                    x?;
                }
            }
        }
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
    fn load_raw() {
        let db = DB::open(":memory:").unwrap();
        let out = db
            .query_raw::<TestSerialize>("select 'x' as id, 1 as integer, 'y' as string")
            .unwrap();
        assert_eq!(
            out[0],
            TestSerialize {
                id: "x".into(),
                integer: 1,
                string: "y".into()
            }
        );
    }

    #[test]
    fn load_store() {
        let db = DB::open(":memory:").unwrap();
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
        let db = DB::open(":memory:").unwrap();
        db.create_table::<TestSerialize>().unwrap();
        let output = db.load::<TestSerialize>("a".into()).unwrap();
        assert_eq!(None, output);
    }
}
