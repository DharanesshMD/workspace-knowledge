use crate::{db, Error};
use sqlx::PgPool;
use std::time::Duration;
#[derive(Clone, Debug)]
pub struct PoolConfig {
    pub max_connections: u32,
    pub acquire_timeout: Duration,
    pub statement_timeout: Duration,
    pub lock_timeout: Duration,
}
impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 10,
            acquire_timeout: Duration::from_secs(5),
            statement_timeout: Duration::from_secs(5),
            lock_timeout: Duration::from_secs(3),
        }
    }
}
impl PoolConfig {
    pub fn validate(&self) -> Result<(), Error> {
        if !(1..=32).contains(&self.max_connections)
            || !(100..=10000).contains(&self.acquire_timeout.as_millis())
            || !(100..=15000).contains(&self.statement_timeout.as_millis())
            || !(100..=5000).contains(&self.lock_timeout.as_millis())
            || self.lock_timeout > self.statement_timeout
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub async fn connect(&self, url: &str) -> Result<PgPool, Error> {
        self.validate()?;
        let statement = format!("{}ms", self.statement_timeout.as_millis());
        let lock = format!("{}ms", self.lock_timeout.as_millis());
        sqlx::postgres::PgPoolOptions::new().max_connections(self.max_connections).acquire_timeout(self.acquire_timeout)
        .after_connect(move|connection,_|{let statement=statement.clone();let lock=lock.clone();Box::pin(async move{
            sqlx::query("SELECT set_config('statement_timeout',$1,false),set_config('lock_timeout',$2,false)").bind(statement).bind(lock).execute(connection).await?;Ok(())
        })}).connect(url).await.map_err(db)
    }
}
