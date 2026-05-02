use anyhow::Result;
use rusqlite::{params, Connection};
use serde::Serialize;

pub const FEATURE_ROUTINES: &str = "routines";

struct FeatureDef {
    id: &'static str,
    label: &'static str,
    description: &'static str,
}

const FEATURE_REGISTRY: &[FeatureDef] = &[FeatureDef {
    id: FEATURE_ROUTINES,
    label: "Routines",
    description: "Recurring scheduled prompts that run automatically on a cron schedule with budget caps and notifications.",
}];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureInfo {
    pub id: String,
    pub enabled: bool,
    pub label: String,
    pub description: String,
}

pub fn is_feature_enabled(conn: &Connection, feature: &str) -> Result<bool> {
    let result = conn.query_row(
        "SELECT enabled FROM features WHERE id = ?1",
        params![feature],
        |row| row.get::<_, bool>(0),
    );
    match result {
        Ok(enabled) => Ok(enabled),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(e) => Err(e.into()),
    }
}

pub fn set_feature_enabled(conn: &Connection, feature: &str, enabled: bool) -> Result<()> {
    if !FEATURE_REGISTRY.iter().any(|f| f.id == feature) {
        anyhow::bail!("unknown feature: {feature}");
    }
    conn.execute(
        "INSERT INTO features (id, enabled) VALUES (?1, ?2)
         ON CONFLICT(id) DO UPDATE SET enabled = excluded.enabled",
        params![feature, enabled],
    )?;
    Ok(())
}

pub fn list_features(conn: &Connection) -> Result<Vec<FeatureInfo>> {
    let mut features = Vec::new();
    for def in FEATURE_REGISTRY {
        let enabled = is_feature_enabled(conn, def.id)?;
        features.push(FeatureInfo {
            id: def.id.to_string(),
            enabled,
            label: def.label.to_string(),
            description: def.description.to_string(),
        });
    }
    Ok(features)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db() -> Connection {
        let conn = Connection::open(":memory:").unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS features (
                id TEXT PRIMARY KEY,
                enabled INTEGER NOT NULL DEFAULT 0
            );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_feature_disabled_by_default() {
        let conn = setup_db();
        assert!(!is_feature_enabled(&conn, FEATURE_ROUTINES).unwrap());
    }

    #[test]
    fn test_enable_feature() {
        let conn = setup_db();
        set_feature_enabled(&conn, FEATURE_ROUTINES, true).unwrap();
        assert!(is_feature_enabled(&conn, FEATURE_ROUTINES).unwrap());
    }

    #[test]
    fn test_disable_feature() {
        let conn = setup_db();
        set_feature_enabled(&conn, FEATURE_ROUTINES, true).unwrap();
        set_feature_enabled(&conn, FEATURE_ROUTINES, false).unwrap();
        assert!(!is_feature_enabled(&conn, FEATURE_ROUTINES).unwrap());
    }

    #[test]
    fn test_unknown_feature_rejected() {
        let conn = setup_db();
        let result = set_feature_enabled(&conn, "nonexistent", true);
        assert!(result.is_err());
    }

    #[test]
    fn test_list_features() {
        let conn = setup_db();
        let features = list_features(&conn).unwrap();
        assert_eq!(features.len(), 1);
        assert_eq!(features[0].id, "routines");
        assert!(!features[0].enabled);
    }

    #[test]
    fn test_set_feature_is_idempotent() {
        let conn = setup_db();
        set_feature_enabled(&conn, FEATURE_ROUTINES, true).unwrap();
        set_feature_enabled(&conn, FEATURE_ROUTINES, true).unwrap();
        assert!(is_feature_enabled(&conn, FEATURE_ROUTINES).unwrap());
    }
}
