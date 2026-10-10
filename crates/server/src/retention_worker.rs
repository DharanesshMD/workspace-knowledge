//! Independent operator process: no HTTP capability and no summary text in logs.
use std::{future::Future, time::Duration};
use wk_knowledge::Knowledge;
pub async fn run(
    k: &Knowledge,
    shutdown: impl Future<Output = ()>,
    period: Duration,
) -> Result<(), String> {
    if period.is_zero() {
        return Err("retention period must be positive".into());
    }
    tokio::pin!(shutdown);
    let mut ticks = tokio::time::interval(period);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {_=&mut shutdown=>return Ok(()),_=ticks.tick()=>{}}
        tokio::select! {
         _=&mut shutdown=>return Ok(()),
         result=k.retention_batch(1000)=>match result {Ok(counts)=>println!("{}",serde_json::to_string(&counts).map_err(|_|"invalid retention counts")?),Err(wk_knowledge::Error::Denied)=>return Err("retention worker requires operator connection".into()),Err(_)=>eprintln!("retention cleanup failed; retrying on next scheduled batch")}
        }
    }
}
