use rusqlite::{params, Connection};

#[test]
fn canonicalization_preserves_both_connections_and_avoids_existing_suffixes() {
    for (legacy, canonical) in [
        ("ollamacloud", "ollama-cloud"),
        ("azureopenai", "azure-openai"),
    ] {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE provider_connections (
            id TEXT PRIMARY KEY, provider_kind TEXT, name TEXT,
            UNIQUE(provider_kind, name));
            CREATE TABLE provider_quota_observations (provider_kind TEXT);",
        )
        .unwrap();
        for (id, kind, name) in [
            ("legacy", legacy, "shared"),
            ("canonical", canonical, "shared"),
            (
                "suffix",
                canonical,
                "shared (migrated 00000000000000000001-0)",
            ),
        ] {
            conn.execute(
                "INSERT INTO provider_connections VALUES (?1, ?2, ?3)",
                params![id, kind, name],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO provider_quota_observations VALUES (?1)",
            [legacy],
        )
        .unwrap();
        conn.execute_batch(include_str!(
            "../src/migrations/V15__provider_descriptor_ids.sql"
        ))
        .unwrap();
        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM provider_connections WHERE provider_kind = ?1",
                [canonical],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rows, 3);
        let name: String = conn
            .query_row(
                "SELECT name FROM provider_connections WHERE id = 'legacy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(name, "shared (migrated 00000000000000000001-1)");
        let quota: String = conn
            .query_row(
                "SELECT provider_kind FROM provider_quota_observations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(quota, canonical);
    }
}
