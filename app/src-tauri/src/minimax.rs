//! MiniMax Code's local runtime projection, read-only and without message content.
use crate::models::SessionDatabase;
use crate::session::{SessionProbe, SessionProbeFailure, Signal};

pub fn probe(database: &SessionDatabase, now: i64) -> (SessionProbe, Option<SessionProbeFailure>) {
    let connection = match crate::sqlite::open_readonly(&database.path) {
        Ok(connection) => connection,
        Err(crate::sqlite::Failure::Missing) => return (SessionProbe::default(), None),
        Err(_) => return (SessionProbe::default(), Some(SessionProbeFailure::UnreadableDatabase("MiniMax 会话库无法读取".into()))),
    };
    // Do not interpret an idle but never-used session as a completed task.
    let result = connection.query_row(
        "SELECT s.session_id, s.status, s.updated_at_ms, (SELECT MAX(ts) FROM local_runtime_token_usage u WHERE u.session_id=s.session_id) FROM local_runtime_sessions s WHERE archived=0 ORDER BY CASE WHEN status='started' AND updated_at_ms >= ?1 THEN 0 ELSE 1 END, updated_at_ms DESC LIMIT 1",
        [now.saturating_sub(300_000)], |row| Ok((row.get::<_,String>(0)?, row.get::<_,String>(1)?, row.get::<_,i64>(2)?, row.get::<_,Option<i64>>(3)?)));
    let (id, status, updated, usage_time) = match result {
        Ok(row) => row,
        Err(rusqlite::Error::QueryReturnedNoRows) => return (SessionProbe::default(), None),
        Err(_) => return (SessionProbe::default(), Some(SessionProbeFailure::UnreadableDatabase("MiniMax 会话库结构与解析器不匹配".into()))),
    };
    let age = now.saturating_sub(updated).max(0);
    let fingerprint = format!("minimax-{id}-{updated}");
    let signal = if status == "started" && age <= 300_000 {
        Some(Signal::Active(fingerprint, None))
    } else if status == "idle" && age <= 60_000 && usage_time.is_some_and(|ts| ts <= updated && updated.saturating_sub(ts) <= 60_000) {
        Some(Signal::Completed(fingerprint))
    } else { None };
    (SessionProbe { signal, ..SessionProbe::default() }, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn active_idle_empty_and_stale_sessions_are_distinct() {
        let sandbox = crate::testutil::Sandbox::new("minimax-state");
        let path = sandbox.path().join("runtime.sqlite");
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE local_runtime_sessions(session_id TEXT,status TEXT,updated_at_ms INTEGER,archived INTEGER);CREATE TABLE local_runtime_token_usage(session_id TEXT,ts INTEGER);").unwrap();
        let db = SessionDatabase { path:path.to_string_lossy().into(),schema:crate::models::SessionSchema::MiniMaxRuntime,status_sql:None };
        connection.execute("INSERT INTO local_runtime_sessions VALUES('fixture','idle',1000000,0)",[]).unwrap();
        assert!(probe(&db,1_000_000).0.signal.is_none(), "unused idle is not completed");
        connection.execute("UPDATE local_runtime_sessions SET status='started'",[]).unwrap();
        assert!(matches!(probe(&db,1_000_001).0.signal,Some(Signal::Active(..))));
        assert!(probe(&db,1_400_001).0.signal.is_none(), "stale started must expire");
        connection.execute("UPDATE local_runtime_sessions SET status='idle'",[]).unwrap();
        connection.execute("INSERT INTO local_runtime_token_usage VALUES('fixture',999999)",[]).unwrap();
        assert!(matches!(probe(&db,1_000_001).0.signal,Some(Signal::Completed(..))));
        connection.execute("UPDATE local_runtime_sessions SET archived=1",[]).unwrap();
        assert!(probe(&db,1_000_001).0.signal.is_none());
    }
}
